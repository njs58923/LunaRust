use anyhow::Result;
use js_runtime::{Engine, TabAction};

#[test]
fn startup_preference_is_a_separate_host_action_from_current_mcp_state() -> Result<()> {
    let mut engine = Engine::new();
    engine.eval("hiperspace.dimention.setMcpAutoStart(true); hiperspace.dimention.setMcpEnabled(false);")?;
    let actions = engine.drain_tab_action_queue();
    assert_eq!(actions.len(), 2);
    assert!(matches!(actions[0], TabAction::SetMcpAutoStart { enabled: true }));
    assert!(matches!(actions[1], TabAction::SetMcpEnabled { enabled: false }));
    Ok(())
}
