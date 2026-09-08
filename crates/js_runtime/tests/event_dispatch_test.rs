use anyhow::Result;
use js_runtime::Engine;

#[test]
fn event_lookup_does_not_enumerate_siblings_and_rejects_detached_targets() -> Result<()> {
    let mut engine = Engine::new();
    engine.update_tag_snapshot((0..=10_000).map(|id| (id, "box".to_owned())).collect());
    engine.update_hierarchy_snapshot(
        (1..=10_000).map(|id| (id, 0)).collect(),
        [(0, (1..=10_000).collect())].into_iter().collect(),
    );
    engine.eval(r#"
        const root = hiperspace.dimention;
        const target = root.children[9999];
        let received = 0, enumerations = 0;
        target.addEventListener('toque', () => received++);
        Object.defineProperty(root, 'children', {
            get() { enumerations++; throw Error('event lookup enumerated siblings'); }
        });
    "#)?;
    engine.push_dom_toque_event(10_000, 0.0, 0.0, 0.0);
    engine.fire_raf(16.0);
    engine.eval("if (received !== 1 || enumerations !== 0) throw Error('event lookup must be independent of sibling count');")?;

    // The cached wrapper and tag still exist, but it is no longer in this document.
    engine.update_hierarchy_snapshot(Default::default(), Default::default());
    engine.push_dom_toque_event(10_000, 0.0, 0.0, 0.0);
    engine.fire_raf(32.0);
    engine.eval("if (received !== 1) throw Error('detached target received input');")?;

    // Malformed hierarchy must terminate without escaping to another document.
    engine.update_hierarchy_snapshot([(10_000, 9999), (9999, 10_000)].into_iter().collect(), Default::default());
    engine.push_dom_toque_event(10_000, 0.0, 0.0, 0.0);
    engine.fire_raf(48.0);
    engine.eval("if (received !== 1) throw Error('cyclic target received input');")?;
    Ok(())
}
