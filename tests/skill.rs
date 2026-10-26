use std::process::Command;

#[test]
fn skill_is_a_self_contained_agent_skill() {
    let output = Command::new(env!("CARGO_BIN_EXE_herdr-app")).arg("--skill").output().unwrap();
    assert!(output.status.success());
    let skill = String::from_utf8(output.stdout).unwrap();
    assert_eq!(skill, include_str!("../src/SKILL.md"));
    assert!(skill.lines().count() < 500, "skill must stay under 500 lines");
    let mut lines = skill.lines();
    assert_eq!(lines.next(), Some("---"));
    assert_eq!(lines.next(), Some("name: herdr-app"));
    let description = lines.next().unwrap().strip_prefix("description: ").unwrap();
    let description: String = serde_json::from_str(description).expect("quoted YAML description");
    assert!(!description.is_empty() && description.chars().count() < 1024);
    assert_eq!(lines.next(), Some("---"));
}
