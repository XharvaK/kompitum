//! The chain of workers, what each one's block means, and what it has spent.
//! Pure: `task.rs` keeps the file and the pump asks these questions.

use crate::task::clock;
use crate::task::quota::{Refusal, Source};
use crate::task::types::Task;
use crate::task::worker::Worker;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Which workers a new task may use, in order.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Auto,
    Cc,
    Go,
    Free,
}

impl Mode {
    pub fn key(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Cc => "cc",
            Mode::Go => "go",
            Mode::Free => "free",
        }
    }

    pub fn from_key(key: &str) -> Option<Mode> {
        match key {
            "auto" => Some(Mode::Auto),
            "cc" => Some(Mode::Cc),
            "go" => Some(Mode::Go),
            "free" => Some(Mode::Free),
            _ => None,
        }
    }
}

/// A worker that may not start until `until_ms`, and who said so.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Block {
    pub until_ms: u64,
    pub window: String,
    pub source: Source,
    /// A person's reason, or the worker's words.
    pub message: String,
    /// The task whose turn was refused.
    pub task: Option<String>,
    /// Credits refusals in a row: the next wait doubles.
    pub refusals: u32,
    pub set_ms: u64,
}

/// `<state>/workers.json`: the mode and every worker's block.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct WorkersFile {
    pub mode: Mode,
    pub blocks: BTreeMap<String, Block>,
}

/// The order a mode tries its workers in.
pub fn chain(mode: Mode) -> Vec<Worker> {
    match mode {
        Mode::Auto => vec![Worker::Cc, Worker::Go],
        Mode::Cc => vec![Worker::Cc],
        Mode::Go => vec![Worker::Go],
        Mode::Free => vec![Worker::Free, Worker::Cc, Worker::Go],
    }
}

/// The chain with this worker first: a follow-up stays with the worker that did the work, while it can.
pub fn stay_with(mut chain: Vec<Worker>, key: &str) -> Vec<Worker> {
    if let Some(at) = chain.iter().position(|worker| worker.key() == key) {
        let worker = chain.remove(at);
        chain.insert(0, worker);
    }
    chain
}

fn limit_words(window: &str) -> &'static str {
    match window {
        "5-hour" => "its 5-hour limit",
        "weekly" => "its weekly limit",
        "monthly" => "its monthly limit",
        "daily" => "its daily limit",
        "rate" => "its rate limit",
        "credits" => "its credits",
        "auth" => "its login",
        _ => "its quota",
    }
}

/// What a block means, in the words `kom workers` and a failed start use.
pub fn sentence(label: &str, block: &Block, now_ms: u64, offset_min: i32) -> String {
    let when = clock::when_words(block.until_ms, now_ms, offset_min);
    match block.source {
        Source::Person => {
            if block.message.is_empty() {
                format!("{label} is out by a person's word, back {when}.")
            } else {
                format!("{label} is out by a person's word ({}), back {when}.", block.message)
            }
        }
        Source::Named => format!("{label} is out of {}, back {when}.", limit_words(&block.window)),
        Source::Estimate => match block.window.as_str() {
            "credits" => format!("{label} is out of credits, reset unknown, next try {when}."),
            "rate" => format!("{label} is rate limited, next try {when}."),
            "auth" => format!("{label} is not signed in, next try {when}."),
            _ => format!("{label} is out of {}, estimated back {when}.", limit_words(&block.window)),
        },
    }
}

/// The first installed worker that is not blocked. A block whose time has
/// passed doesn't block, and stays in the file for its `refusals`.
pub fn pick(
    chain: &[Worker],
    file: &WorkersFile,
    installed: impl Fn(Worker) -> bool,
    now_ms: u64,
    offset_min: i32,
) -> Result<Worker, String> {
    let mut reasons = Vec::new();
    for worker in chain {
        if let Some(block) = file.blocks.get(worker.key()).filter(|block| block.until_ms > now_ms) {
            reasons.push(sentence(worker.label(), block, now_ms, offset_min));
            continue;
        }
        if !installed(*worker) {
            reasons.push(format!("{} is not installed.", worker.label()));
            continue;
        }
        return Ok(*worker);
    }
    if chain.len() == 1
        && let Some(block) = chain.first().and_then(|worker| file.blocks.get(worker.key()))
        && block.until_ms > now_ms
    {
        return Err(format!("QUOTA EXHAUSTED: {}", sentence(chain[0].label(), block, now_ms, offset_min)));
    }
    Err(format!("ALL WORKERS OUT: {}", reasons.join(" ")))
}

/// Whether a worker of this chain is out only until a time: a stranded task
/// with one may go on by itself when that block lapses.
pub fn may_come_back(chain: &[Worker], file: &WorkersFile, now_ms: u64) -> bool {
    chain.iter().any(|worker| file.blocks.get(worker.key()).is_some_and(|block| block.until_ms > now_ms))
}

/// The block a refusal makes, in the trust order person, named, estimate.
/// None means nothing changes.
pub fn after_refusal(current: Option<&Block>, refusal: &Refusal, task: Option<&str>, now_ms: u64) -> Option<Block> {
    if let Some(current) = current.filter(|block| block.until_ms > now_ms) {
        if current.source == Source::Person {
            return None;
        }
        if current.source == Source::Named && refusal.source == Source::Estimate {
            return None;
        }
    }
    let mut block = Block {
        until_ms: refusal.until_ms,
        window: refusal.window.clone(),
        source: refusal.source,
        message: refusal.message.clone(),
        task: task.map(String::from),
        refusals: 0,
        set_ms: now_ms,
    };
    if refusal.source == Source::Estimate && refusal.window == "credits" {
        let prior = current
            .filter(|block| block.source == Source::Estimate && block.window == "credits")
            .map_or(0, |block| block.refusals);
        let refusals = prior + 1;
        let hours = 1u64.checked_shl(refusals.saturating_sub(1)).unwrap_or(u64::MAX);
        let wait = hours.saturating_mul(3_600_000).min(7 * 86_400_000);
        block.refusals = refusals;
        block.until_ms = now_ms.saturating_add(wait);
    }
    Some(block)
}

/// A good turn clears the worker's block, unless a person's is in force.
pub fn after_good_turn(file: &mut WorkersFile, key: &str) {
    if file.blocks.get(key).is_some_and(|block| block.source != Source::Person) {
        file.blocks.remove(key);
    }
}

/// What one worker spent over the last 5 hours, 7 days and 30 days.
#[derive(Serialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct Spend {
    pub five_hours: Bucket,
    pub week: Bucket,
    pub month: Bucket,
}

#[derive(Serialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct Bucket {
    pub runs: u64,
    pub cost_usd: f64,
}

/// Each turn counts by its `ended_ms` and worker. Discarded tasks count too:
/// spent is spent.
pub fn spend(tasks: &[Task], now_ms: u64) -> BTreeMap<String, Spend> {
    let mut totals: BTreeMap<String, Spend> = BTreeMap::new();
    for task in tasks {
        for turn in &task.turns {
            let Some(ended) = turn.ended_ms else { continue };
            if ended > now_ms {
                continue;
            }
            let entry = totals.entry(turn.worker.clone()).or_default();
            let age = now_ms - ended;
            for (bucket, length) in [
                (&mut entry.five_hours, 5 * 3_600_000),
                (&mut entry.week, 7 * 86_400_000),
                (&mut entry.month, 30 * 86_400_000),
            ] {
                if age <= length {
                    bucket.runs += 1;
                    bucket.cost_usd += turn.usage.cost_usd;
                }
            }
        }
    }
    for spend in totals.values_mut() {
        for bucket in [&mut spend.five_hours, &mut spend.week, &mut spend.month] {
            bucket.cost_usd = (bucket.cost_usd * 1_000_000.0).round() / 1_000_000.0;
        }
    }
    totals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_follow_up_stays_with_its_worker_and_only_within_the_chain() {
        assert_eq!(stay_with(chain(Mode::Auto), "go"), [Worker::Go, Worker::Cc]);
        assert_eq!(stay_with(chain(Mode::Auto), "cc"), [Worker::Cc, Worker::Go]);
        assert_eq!(stay_with(chain(Mode::Go), "cc"), [Worker::Go]);
    }
    use crate::task::clock::Civil;
    use crate::task::types::{Turn, Usage};

    fn now() -> u64 {
        clock::to_ms(&Civil { year: 2026, month: 11, day: 5, hour: 9, minute: 0, second: 0, weekday: 0 }, 0)
    }

    fn block(source: Source, window: &str, until_ms: u64) -> Block {
        Block { until_ms, window: window.into(), source, message: String::new(), ..Default::default() }
    }

    fn refusal(window: &str, source: Source, until_ms: u64) -> Refusal {
        Refusal { window: window.into(), until_ms, source, message: "quota".into() }
    }

    fn workers_file(blocks: &[(&str, Block)]) -> WorkersFile {
        WorkersFile {
            mode: Mode::Auto,
            blocks: blocks.iter().map(|(key, block)| (key.to_string(), block.clone())).collect(),
        }
    }

    #[test]
    fn auto_passes_over_a_blocked_worker_and_one_not_installed() {
        let now = now();
        let file = workers_file(&[("cc", block(Source::Named, "weekly", now + 3_600_000))]);
        let installed = |worker: Worker| worker != Worker::Free;
        assert_eq!(pick(&chain(Mode::Auto), &file, installed, now, 0), Ok(Worker::Go));
        let empty = WorkersFile::default();
        let none = |_worker: Worker| false;
        assert_eq!(
            pick(&chain(Mode::Auto), &empty, none, now, 0),
            Err("ALL WORKERS OUT: Command Code is not installed. opencode Go is not installed.".into())
        );
        let cc_only = workers_file(&[("cc", block(Source::Named, "weekly", now + 3_600_000))]);
        assert_eq!(
            pick(&chain(Mode::Cc), &cc_only, installed, now, 0),
            Err("QUOTA EXHAUSTED: Command Code is out of its weekly limit, back at 10:00.".into())
        );
    }

    #[test]
    fn free_goes_on_to_command_code_then_go() {
        let now = now();
        let both = workers_file(&[
            ("free", block(Source::Named, "daily", now + 3_600_000)),
            ("cc", block(Source::Named, "weekly", now + 3_600_000)),
        ]);
        assert_eq!(pick(&chain(Mode::Free), &both, |_| true, now, 0), Ok(Worker::Go));
        let cc_out = workers_file(&[("free", block(Source::Named, "daily", now + 3_600_000))]);
        assert_eq!(pick(&chain(Mode::Free), &cc_out, |_| true, now, 0), Ok(Worker::Cc));
        assert_eq!(chain(Mode::Free), [Worker::Free, Worker::Cc, Worker::Go]);
        assert_eq!(chain(Mode::Auto), [Worker::Cc, Worker::Go]);
    }

    #[test]
    fn a_block_lapses_by_itself() {
        let now = now();
        let file = workers_file(&[("cc", block(Source::Named, "weekly", now - 1))]);
        assert_eq!(pick(&chain(Mode::Auto), &file, |_| true, now, 0), Ok(Worker::Cc));
        assert!(file.blocks.contains_key("cc"), "a lapsed block stays for its refusals");
    }

    #[test]
    fn a_stranded_chain_comes_back_only_while_a_block_is_live() {
        let now = now();
        let live = workers_file(&[("cc", block(Source::Named, "weekly", now + 1))]);
        assert!(may_come_back(&chain(Mode::Auto), &live, now));
        let lapsed = workers_file(&[("cc", block(Source::Named, "weekly", now - 1))]);
        assert!(!may_come_back(&chain(Mode::Auto), &lapsed, now));
        assert!(!may_come_back(&chain(Mode::Auto), &WorkersFile::default(), now));
    }

    #[test]
    fn a_persons_block_is_not_replaced_by_a_refusal_until_its_time_passes() {
        let now = now();
        let person = block(Source::Person, "", now + 86_400_000);
        let estimate = refusal("weekly", Source::Estimate, now + 3_600_000);
        assert_eq!(after_refusal(Some(&person), &estimate, Some("t"), now), None);
        let named = refusal("weekly", Source::Named, now + 3_600_000);
        assert_eq!(after_refusal(Some(&person), &named, Some("t"), now), None);
        let later = now + 86_400_000;
        assert!(after_refusal(Some(&person), &named, Some("t"), later).is_some());
    }

    #[test]
    fn a_named_reset_replaces_an_estimate_and_not_the_other_way() {
        let now = now();
        let estimate = block(Source::Estimate, "weekly", now + 3_600_000);
        let named = refusal("weekly", Source::Named, now + 7 * 86_400_000);
        let replaced = after_refusal(Some(&estimate), &named, Some("t"), now).unwrap();
        assert_eq!(replaced.source, Source::Named);
        assert_eq!(replaced.until_ms, now + 7 * 86_400_000);
        let named_block = block(Source::Named, "weekly", now + 7 * 86_400_000);
        assert_eq!(
            after_refusal(Some(&named_block), &refusal("weekly", Source::Estimate, now + 1), Some("t"), now),
            None
        );
        let fresh = after_refusal(Some(&estimate), &refusal("weekly", Source::Estimate, now + 1), Some("t"), now);
        assert_eq!(fresh.unwrap().source, Source::Estimate);
    }

    #[test]
    fn each_credits_refusal_doubles_the_wait_up_to_a_week() {
        let now = now();
        let mut current: Option<Block> = None;
        let mut waits = Vec::new();
        for n in 1..=5 {
            let next =
                after_refusal(current.as_ref(), &refusal("credits", Source::Estimate, now + 3_600_000), Some("t"), now)
                    .unwrap();
            assert_eq!(next.refusals, n);
            waits.push(next.until_ms - now);
            current = Some(next);
        }
        assert_eq!(waits, [3_600_000, 7_200_000, 14_400_000, 28_800_000, 57_600_000]);
        let long = after_refusal(
            Some(&block(Source::Estimate, "credits", now - 1)),
            &refusal("credits", Source::Estimate, now + 1),
            Some("t"),
            now,
        )
        .unwrap();
        assert_eq!(long.refusals, 1, "a block that has lapsed still counts");
        let capped = Block { refusals: 40, ..block(Source::Estimate, "credits", now - 1) };
        let capped =
            after_refusal(Some(&capped), &refusal("credits", Source::Estimate, now + 1), Some("t"), now).unwrap();
        assert_eq!(capped.until_ms - now, 7 * 86_400_000);
    }

    #[test]
    fn a_good_turn_clears_the_count_but_not_a_persons_block() {
        let mut file = workers_file(&[("cc", block(Source::Estimate, "credits", 10))]);
        after_good_turn(&mut file, "cc");
        assert!(!file.blocks.contains_key("cc"));
        let mut file = workers_file(&[("cc", block(Source::Person, "", 10))]);
        after_good_turn(&mut file, "cc");
        assert!(file.blocks.contains_key("cc"));
        after_good_turn(&mut file, "go");
        assert!(file.blocks.contains_key("cc"));
    }

    #[test]
    fn the_sentences_are_word_for_word() {
        let now = now();
        let at = now + 5 * 3_600_000;
        let person = Block { message: "monthly credits dead".into(), ..block(Source::Person, "", at) };
        assert_eq!(
            sentence("Command Code", &person, now, 0),
            "Command Code is out by a person's word (monthly credits dead), back at 14:00."
        );
        let bare = block(Source::Person, "", at);
        assert_eq!(sentence("Command Code", &bare, now, 0), "Command Code is out by a person's word, back at 14:00.");
        for (window, words) in [
            ("5-hour", "its 5-hour limit"),
            ("weekly", "its weekly limit"),
            ("monthly", "its monthly limit"),
            ("daily", "its daily limit"),
            ("rate", "its rate limit"),
            ("unknown", "its quota"),
        ] {
            assert_eq!(
                sentence("Command Code", &block(Source::Named, window, at), now, 0),
                format!("Command Code is out of {words}, back at 14:00.")
            );
        }
        assert_eq!(
            sentence("Command Code", &block(Source::Estimate, "weekly", at), now, 0),
            "Command Code is out of its weekly limit, estimated back at 14:00."
        );
        assert_eq!(
            sentence("Command Code", &block(Source::Estimate, "credits", at), now, 0),
            "Command Code is out of credits, reset unknown, next try at 14:00."
        );
        assert_eq!(
            sentence("Command Code", &block(Source::Estimate, "rate", at), now, 0),
            "Command Code is rate limited, next try at 14:00."
        );
        assert_eq!(
            sentence("Command Code", &block(Source::Estimate, "auth", at), now, 0),
            "Command Code is not signed in, next try at 14:00."
        );
    }

    #[test]
    fn spend_counts_each_turn_once_in_the_windows_it_falls_in() {
        let now = now();
        let turn = |ended: u64, worker: &str, cost: f64| Turn {
            worker: worker.into(),
            ended_ms: Some(ended),
            usage: Usage { cost_usd: cost, ..Default::default() },
            ..Default::default()
        };
        let tasks = vec![
            Task {
                turns: vec![turn(now - 3_600_000, "cc", 1.0), turn(now - 8 * 86_400_000, "cc", 2.0)],
                ..Default::default()
            },
            Task { turns: vec![turn(now - 20 * 86_400_000, "go", 4.0)], ..Default::default() },
            Task {
                state: crate::task::types::TaskState::Discarded,
                turns: vec![turn(now - 60_000, "go", 0.5)],
                ..Default::default()
            },
        ];
        let spend = spend(&tasks, now);
        assert_eq!(spend["cc"].five_hours.runs, 1);
        assert_eq!(spend["cc"].week.runs, 1);
        assert_eq!(spend["cc"].week.cost_usd, 1.0);
        assert_eq!(spend["cc"].month.runs, 2);
        assert_eq!(spend["cc"].month.cost_usd, 3.0);
        assert_eq!(spend["go"].five_hours.cost_usd, 0.5, "a discarded task still spent");
        assert_eq!(spend["go"].month.cost_usd, 4.5);
    }
}
