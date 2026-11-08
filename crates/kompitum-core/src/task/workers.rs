//! The chain of workers, what each one's block means, and what it has spent.
//! Pure: `task.rs` keeps the file and the pump asks these questions.

use crate::rpc::RpcError;
use crate::task::clock;
use crate::task::quota::{Refusal, Source};
use crate::task::types::Task;
use crate::task::worker::{Worker, builtin};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The auto chain when the file names none.
const DEFAULT_ORDER: [&str; 2] = ["cc", "go"];

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

/// `<state>/workers.json`: the mode, the person's workers, the auto chain, and every worker's block.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct WorkersFile {
    /// "auto", "free", or any known worker key.
    pub mode: String,
    pub blocks: BTreeMap<String, Block>,
    /// The workers the person added; the built-ins are never stored.
    #[serde(default)]
    pub workers: Vec<Worker>,
    /// The auto chain; empty means the default.
    #[serde(default)]
    pub order: Vec<String>,
}

impl Default for WorkersFile {
    fn default() -> Self {
        WorkersFile { mode: "auto".into(), blocks: BTreeMap::new(), workers: Vec::new(), order: Vec::new() }
    }
}

/// The built-ins, then the person's workers.
pub fn all(file: &WorkersFile) -> Vec<Worker> {
    let mut workers = builtin();
    workers.extend(file.workers.iter().cloned());
    workers
}

fn choices(file: &WorkersFile) -> String {
    all(file).iter().map(|worker| worker.key.clone()).collect::<Vec<_>>().join(", ")
}

fn unknown_worker(file: &WorkersFile, key: &str) -> RpcError {
    RpcError::invalid_params(format!("unknown worker {key:?}: use {}", choices(file)))
}

/// Look a worker key up in the built-ins and the person's workers.
pub fn find(file: &WorkersFile, key: &str) -> Result<Worker, RpcError> {
    all(file).into_iter().find(|worker| worker.key == key).ok_or_else(|| unknown_worker(file, key))
}

/// The auto chain, with the default filled in.
pub fn order(file: &WorkersFile) -> Vec<String> {
    if file.order.is_empty() { DEFAULT_ORDER.iter().map(|key| key.to_string()).collect() } else { file.order.clone() }
}

fn no_known_order(file: &WorkersFile) -> String {
    format!("workers.json names no known worker in its order ({})", order(file).join(", "))
}

/// The order a mode tries its workers in.
pub fn chain(file: &WorkersFile) -> Vec<Worker> {
    chain_for(file, &file.mode).unwrap_or_default()
}

/// The chain one mode names: auto is the order, free comes first, and any other key is alone.
pub fn chain_for(file: &WorkersFile, mode: &str) -> Result<Vec<Worker>, RpcError> {
    match mode {
        "auto" => {
            let chain: Vec<Worker> = order(file).iter().filter_map(|key| find(file, key).ok()).collect();
            if chain.is_empty() { Err(RpcError::invalid_params(no_known_order(file))) } else { Ok(chain) }
        }
        "free" => {
            let mut chain: Vec<Worker> = find(file, "free").ok().into_iter().collect();
            chain
                .extend(order(file).iter().filter(|key| key.as_str() != "free").filter_map(|key| find(file, key).ok()));
            Ok(chain)
        }
        key => Ok(vec![find(file, key)?]),
    }
}

/// Validate and add a worker, then put it at the end of the auto chain.
pub fn add_worker(file: &mut WorkersFile, worker: Worker) -> Result<(), RpcError> {
    crate::addr::check_name(&worker.key)?;
    if worker.key == "auto" {
        return Err(RpcError::invalid_params("\"auto\" is the mode word: pick another key"));
    }
    if all(file).iter().any(|known| known.key == worker.key) {
        return Err(RpcError::invalid_params(format!("worker {:?} is already taken", worker.key)));
    }
    if worker.route.trim().is_empty() {
        return Err(RpcError::invalid_params("a worker needs a route"));
    }
    if file.order.is_empty() {
        file.order = DEFAULT_ORDER.iter().map(|key| key.to_string()).collect();
    }
    file.order.retain(|key| key != &worker.key);
    file.order.push(worker.key.clone());
    file.workers.push(worker);
    Ok(())
}

/// Remove an added worker, its block, and its place in the auto chain.
pub fn remove_worker(file: &mut WorkersFile, key: &str) -> Result<(), RpcError> {
    if builtin().iter().any(|worker| worker.key == key) {
        return Err(RpcError::invalid_params(format!("{key} is built in: leave it out of the order instead")));
    }
    let Some(at) = file.workers.iter().position(|worker| worker.key == key) else {
        return Err(unknown_worker(file, key));
    };
    file.workers.remove(at);
    file.blocks.remove(key);
    file.order.retain(|item| item != key);
    if file.mode == key {
        file.mode = "auto".into();
    }
    Ok(())
}

/// A new auto chain: known keys, no repeats, at least one.
pub fn set_order(file: &mut WorkersFile, order: Vec<String>) -> Result<(), RpcError> {
    if order.is_empty() {
        return Err(RpcError::invalid_params("an order needs at least one worker key"));
    }
    let mut seen = BTreeSet::new();
    for key in &order {
        find(file, key)?;
        if !seen.insert(key.as_str()) {
            return Err(RpcError::invalid_params(format!("worker {key:?} is in the order twice")));
        }
    }
    file.order = order;
    Ok(())
}

/// A mode: auto, or one known worker key.
pub fn set_mode(file: &mut WorkersFile, mode: &str) -> Result<(), RpcError> {
    if mode == "auto" {
        file.mode = "auto".into();
        return Ok(());
    }
    find(file, mode)?;
    file.mode = mode.to_string();
    Ok(())
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
    installed: impl Fn(&Worker) -> bool,
    now_ms: u64,
    offset_min: i32,
) -> Result<Worker, String> {
    if chain.is_empty() {
        return Err(no_known_order(file));
    }
    let mut reasons = Vec::new();
    for worker in chain {
        if let Some(block) = file.blocks.get(worker.key()).filter(|block| block.until_ms > now_ms) {
            reasons.push(sentence(worker.label(), block, now_ms, offset_min));
            continue;
        }
        if !installed(worker) {
            reasons.push(format!("{} is not installed.", worker.label()));
            continue;
        }
        return Ok(worker.clone());
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

    use crate::task::clock::Civil;
    use crate::task::types::{Turn, Usage};
    use crate::task::worker::Cli;

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
            blocks: blocks.iter().map(|(key, block)| (key.to_string(), block.clone())).collect(),
            ..Default::default()
        }
    }

    fn custom(key: &str, cli: Cli, route: &str) -> Worker {
        Worker { key: key.into(), label: key.into(), cli, route: route.into() }
    }

    fn keys(chain: &[Worker]) -> Vec<&str> {
        chain.iter().map(Worker::key).collect()
    }

    #[test]
    fn a_follow_up_stays_with_its_worker_and_only_within_the_chain() {
        let mut file = WorkersFile::default();
        assert_eq!(keys(&stay_with(chain(&file), "go")), ["go", "cc"]);
        assert_eq!(keys(&stay_with(chain(&file), "cc")), ["cc", "go"]);
        file.mode = "go".into();
        assert_eq!(keys(&stay_with(chain(&file), "cc")), ["go"]);
    }

    #[test]
    fn the_chain_is_the_order_free_or_one_pinned_worker() {
        let mut file = WorkersFile::default();
        assert_eq!(keys(&chain(&file)), ["cc", "go"], "the default order");
        assert_eq!(keys(&chain_for(&file, "free").unwrap()), ["free", "cc", "go"]);
        file.mode = "go".into();
        assert_eq!(keys(&chain(&file)), ["go"], "a pinned key");
        file.order = vec!["free".into(), "cc".into()];
        file.mode = "auto".into();
        assert_eq!(keys(&chain(&file)), ["free", "cc"], "a custom order");
        file.mode = "free".into();
        assert_eq!(keys(&chain(&file)), ["free", "cc"], "free, then the order without it");

        let mine = custom("mine", Cli::Opencode, "opencode-go/other#max");
        add_worker(&mut file, mine.clone()).unwrap();
        assert_eq!(file.order, ["free", "cc", "mine"], "an added worker goes to the end");
        file.mode = "auto".into();
        assert_eq!(keys(&chain(&file)), ["free", "cc", "mine"]);
        file.mode = "mine".into();
        assert_eq!(chain(&file), [mine], "a pinned added worker is alone");
    }

    #[test]
    fn an_order_with_no_known_worker_is_said_in_words() {
        let mut file = WorkersFile { order: vec!["gone".into()], ..Default::default() };
        assert_eq!(
            chain_for(&file, "auto").unwrap_err().message,
            "workers.json names no known worker in its order (gone)"
        );
        assert_eq!(
            pick(&[], &file, |_| true, now(), 0).unwrap_err(),
            "workers.json names no known worker in its order (gone)",
            "never `ALL WORKERS OUT:` with nothing after it"
        );
        file.order = vec!["gone".into(), "cc".into()];
        assert_eq!(keys(&chain_for(&file, "auto").unwrap()), ["cc"], "the known keys stay");
    }

    #[test]
    fn an_old_workers_file_loads_unchanged() {
        let file: WorkersFile = serde_json::from_str(r#"{"mode":"cc","blocks":{}}"#).unwrap();
        assert_eq!(file.mode, "cc");
        assert_eq!(keys(&chain(&file)), ["cc"]);
        assert!(file.workers.is_empty() && file.order.is_empty());
    }

    #[test]
    fn an_unknown_key_is_refused_with_the_known_ones() {
        let file = WorkersFile::default();
        assert_eq!(
            find(&file, "x").unwrap_err().message,
            r#"unknown worker "x": use cc, go, free"#,
            "the known keys are listed"
        );
    }

    #[test]
    fn the_order_needs_known_keys_no_repeats_and_one_entry() {
        let mut file = WorkersFile::default();
        assert!(set_order(&mut file, vec!["nope".into()]).is_err(), "an unknown key");
        assert!(set_order(&mut file, vec!["cc".into(), "cc".into()]).is_err(), "a repeat");
        assert_eq!(set_order(&mut file, Vec::new()).unwrap_err().message, "an order needs at least one worker key");
        assert!(file.order.is_empty(), "nothing was changed");
        set_order(&mut file, vec!["go".into(), "cc".into()]).unwrap();
        assert_eq!(file.order, ["go", "cc"]);
    }

    #[test]
    fn adding_refuses_taken_bad_keys_and_an_empty_route_and_appends() {
        let mut file = WorkersFile::default();
        let taken = add_worker(&mut file, custom("cc", Cli::CommandCode, "commandcode/x#max")).unwrap_err();
        assert!(taken.message.contains("already taken"), "{}", taken.message);
        let bad = add_worker(&mut file, custom("Bad Key", Cli::Opencode, "opencode/x#max")).unwrap_err();
        assert!(bad.message.contains("bad name"), "{}", bad.message);
        let mode_word = add_worker(&mut file, custom("auto", Cli::Opencode, "opencode/x#max")).unwrap_err();
        assert_eq!(mode_word.message, "\"auto\" is the mode word: pick another key");
        let empty = add_worker(&mut file, custom("mine", Cli::Opencode, "  ")).unwrap_err();
        assert!(empty.message.contains("route"), "{}", empty.message);
        assert!(file.workers.is_empty() && file.order.is_empty(), "nothing was changed");

        add_worker(&mut file, custom("mine", Cli::Opencode, "opencode/x#max")).unwrap();
        assert_eq!(file.order, ["cc", "go", "mine"], "an empty order is filled with the default first");
        file.order = vec!["go".into()];
        add_worker(&mut file, custom("other", Cli::Opencode, "opencode/y#max")).unwrap();
        assert_eq!(file.order, ["go", "other"]);
    }

    #[test]
    fn removing_a_builtin_is_refused_and_an_added_one_leaves_everything() {
        let mut file = WorkersFile::default();
        assert_eq!(
            remove_worker(&mut file, "cc").unwrap_err().message,
            "cc is built in: leave it out of the order instead"
        );
        add_worker(&mut file, custom("mine", Cli::Opencode, "opencode/x#max")).unwrap();
        file.mode = "mine".into();
        file.blocks.insert("mine".into(), block(Source::Person, "", 10));
        remove_worker(&mut file, "mine").unwrap();
        assert!(file.workers.is_empty(), "its worker went");
        assert!(!file.order.iter().any(|key| key == "mine"), "its place in the order went");
        assert!(!file.blocks.contains_key("mine"), "its block went");
        assert_eq!(file.mode, "auto", "a mode that named it goes back to auto");
    }

    #[test]
    fn a_mode_is_auto_or_a_known_key() {
        let mut file = WorkersFile::default();
        set_mode(&mut file, "go").unwrap();
        assert_eq!(file.mode, "go");
        assert!(set_mode(&mut file, "nope").is_err());
        add_worker(&mut file, custom("mine", Cli::Opencode, "opencode/x#max")).unwrap();
        set_mode(&mut file, "mine").unwrap();
        assert_eq!(file.mode, "mine");
    }

    #[test]
    fn auto_passes_over_a_blocked_worker_and_one_not_installed() {
        let now = now();
        let file = workers_file(&[("cc", block(Source::Named, "weekly", now + 3_600_000))]);
        let installed = |worker: &Worker| worker.key != "free";
        assert_eq!(pick(&chain(&file), &file, installed, now, 0), Ok(find(&file, "go").unwrap()));
        let empty = WorkersFile::default();
        let none = |_worker: &Worker| false;
        assert_eq!(
            pick(&chain(&empty), &empty, none, now, 0),
            Err("ALL WORKERS OUT: Command Code is not installed. opencode Go is not installed.".into())
        );
        let cc_only = workers_file(&[("cc", block(Source::Named, "weekly", now + 3_600_000))]);
        let mut pinned = cc_only.clone();
        pinned.mode = "cc".into();
        assert_eq!(
            pick(&chain(&pinned), &cc_only, installed, now, 0),
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
        let mut free = both.clone();
        free.mode = "free".into();
        assert_eq!(pick(&chain(&free), &both, |_| true, now, 0), Ok(find(&both, "go").unwrap()));
        let cc_out = workers_file(&[("free", block(Source::Named, "daily", now + 3_600_000))]);
        assert_eq!(pick(&chain(&free), &cc_out, |_| true, now, 0), Ok(find(&cc_out, "cc").unwrap()));
        assert_eq!(keys(&chain(&free)), ["free", "cc", "go"]);
        assert_eq!(keys(&chain(&WorkersFile::default())), ["cc", "go"]);
    }

    #[test]
    fn pick_can_choose_an_added_worker() {
        let now = now();
        let mut file = WorkersFile::default();
        let mine = custom("mine", Cli::Opencode, "opencode-go/other#max");
        add_worker(&mut file, mine.clone()).unwrap();
        assert_eq!(pick(&chain(&file), &file, |worker| worker.key == "mine", now, 0), Ok(mine));
    }

    #[test]
    fn a_block_lapses_by_itself() {
        let now = now();
        let file = workers_file(&[("cc", block(Source::Named, "weekly", now - 1))]);
        assert_eq!(pick(&chain(&file), &file, |_| true, now, 0), Ok(find(&file, "cc").unwrap()));
        assert!(file.blocks.contains_key("cc"), "a lapsed block stays for its refusals");
    }

    #[test]
    fn a_stranded_chain_comes_back_only_while_a_block_is_live() {
        let now = now();
        let live = workers_file(&[("cc", block(Source::Named, "weekly", now + 1))]);
        assert!(may_come_back(&chain(&live), &live, now));
        let lapsed = workers_file(&[("cc", block(Source::Named, "weekly", now - 1))]);
        assert!(!may_come_back(&chain(&lapsed), &lapsed, now));
        let empty = WorkersFile::default();
        assert!(!may_come_back(&chain(&empty), &empty, now));
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
