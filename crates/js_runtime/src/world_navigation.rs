//! Explicit world navigation, separate from location's include-local navigation.
use deno_core::{op2, OpState};

#[derive(Default)]
pub struct WorldNavigationQueue(pub Vec<String>);

#[op2(fast)]
pub fn op_navigate_world(state: &mut OpState, #[string] url: &str) -> Result<(), anyhow::Error> {
    anyhow::ensure!(!url.trim().is_empty() && url.len() <= 16384, "Invalid world navigation URL");
    let queue = state.borrow_mut::<WorldNavigationQueue>();
    // Only the final destination from this worker tick can be displayed.
    queue.0.clear();
    queue.0.push(url.into());
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn explicit_navigation_is_separate_and_bounded() {
        let mut engine = crate::Engine::new();
        engine.eval("hiperspace.world.navigate('/a'); hiperspace.world.navigate('/b#entry=door');").unwrap();
        assert_eq!(engine.drain_world_navigation(), vec!["/b#entry=door"]);
        assert!(engine.drain_navigate_queue().is_empty());
        assert!(engine.eval("hiperspace.world.navigate('')").is_err());
        assert!(engine.eval("hiperspace.world.navigate({entry:'x'})").is_err());
        assert!(engine.drain_world_navigation().is_empty());
    }
}
