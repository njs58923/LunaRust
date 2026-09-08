use anyhow::Result;
use js_runtime::Engine;

#[test]
fn queued_hover_changes_do_not_overtake_earlier_clicks() -> Result<()> {
    let mut engine = Engine::new();
    engine.update_tag_snapshot([(0, "space".into()), (1, "box".into()), (2, "box".into())].into_iter().collect());
    engine.update_hierarchy_snapshot([(1, 0), (2, 0)].into_iter().collect(), [(0, vec![1, 2])].into_iter().collect());
    engine.eval(r#"
        const [a, b] = hiperspace.dimention.children;
        const received = [];
        a.addEventListener('toque', () => received.push([a.matches(':hover'), b.matches(':hover')]));
        b.addEventListener('toque', () => received.push([a.matches(':hover'), b.matches(':hover')]));
    "#)?;
    engine.push_dom_event("__luna_hover", 1, Some(0.0), None, None);
    engine.push_dom_toque_event(1, 0.0, 0.0, 0.0);
    engine.push_dom_event("__luna_hover", 2, Some(0.0), None, None);
    engine.push_dom_toque_event(2, 0.0, 0.0, 0.0);
    engine.push_dom_event("__luna_hover", -1, Some(0.0), None, None);
    engine.fire_raf(16.0);
    engine.eval(r#"
        if (JSON.stringify(received) !== '[[true,false],[false,true]]') throw Error('later hover overtook click');
        if (a.matches(':hover') || b.matches(':hover')) throw Error('trailing hover was lost');
    "#)?;
    Ok(())
}

#[test]
fn hover_host_snapshot_is_applied_before_click_in_the_worker_tick() -> Result<()> {
    let mut engine = Engine::new();
    engine.update_tag_snapshot([(0, "space".into()), (1, "box".into())].into_iter().collect());
    engine.update_hierarchy_snapshot([(1, 0)].into_iter().collect(), [(0, vec![1])].into_iter().collect());
    engine.eval(r#"
        const box = hiperspace.dimention.children[0];
        let enters = 0, clicks = 0, correct = false;
        box.onpointerenter = () => enters++;
        box.addEventListener('toque', () => { clicks++; correct = box.matches(':hover'); });
    "#)?;
    engine.push_dom_event("__luna_hover", 1, Some(0.0), None, None);
    engine.push_dom_event("__luna_hover", -1, Some(1.0), None, None);
    engine.push_dom_event("__luna_hover", -1, Some(2.0), None, None);
    engine.push_dom_toque_event(1, 0.0, 0.0, 0.0);
    engine.fire_raf(16.0);
    engine.eval("if (enters !== 1 || clicks !== 1 || !correct) throw Error('input ordering');")?;
    engine.push_dom_event("__luna_hover", -1, Some(0.0), None, None);
    engine.fire_raf(32.0);
    engine.eval("if (box.matches(':hover')) throw Error('lost pointer exit');")?;
    Ok(())
}

#[test]
fn hover_boundaries_bubble_without_false_parent_leave_and_preserve_multiple_rays() -> Result<()> {
    let mut engine = Engine::new();
    engine.update_tag_snapshot([(0, "space"), (1, "group"), (2, "box"), (3, "image")]
        .into_iter().map(|(id, tag)| (id, tag.to_owned())).collect());
    engine.update_hierarchy_snapshot(
        [(1, 0), (2, 1), (3, 1)].into_iter().collect(),
        [(0, vec![1]), (1, vec![2, 3])].into_iter().collect(),
    );
    engine.eval(r#"
        const root = hiperspace.dimention;
        const parent = root.children[0], a = parent.children[0], b = parent.children[1];
        const assert = (value, message) => { if (!value) throw Error(message); };
        let parentEnter = 0, parentLeave = 0, rootOver = 0, aLeave = 0;
        parent.onmouseenter = () => parentEnter++;
        parent.onmouseleave = () => parentLeave++;
        root.onmouseover = e => { rootOver++; assert(e.currentTarget === root, 'currentTarget'); };
        a.onmouseleave = e => { aLeave++; assert(e.relatedTarget === b, 'relatedTarget'); };
        __luna_set_hover_targets([2, null, null]);
        assert(a.matches(':hover') && parent.matches(':hover') && root.matches(':hover'), 'ancestor hover');
        __luna_set_hover_targets([2, null, null]);
        assert(parentEnter === 1 && rootOver === 1, 'stationary pointer duplicated events');
        __luna_set_hover_targets([3, null, null]);
        assert(aLeave === 1 && parentLeave === 0 && parentEnter === 1 && rootOver === 2, 'sibling transition');
        assert(!a.matches(':hover') && b.matches(':hover'), 'sibling state');
        __luna_set_hover_targets([3, 3, null]);
        __luna_set_hover_targets([null, 3, null]);
        assert(b.matches(':hover'), 'second ray must retain hover');
        __luna_set_hover_targets([null, null, null]);
        assert(!root.matches(':hover') && !b.matches(':hover'), 'clear hover');
        let stopped = 0;
        b.onpointerover = e => { e.stopImmediatePropagation(); };
        parent.onpointerover = () => stopped++;
        __luna_set_hover_targets([null, null, 3]);
        assert(stopped === 0, 'stopPropagation');
        globalThis.__luna_pending_hover_targets = [null, null, 3];
    "#)?;
    // Removal/foreign hierarchy must release even while the pointer is stationary.
    engine.update_hierarchy_snapshot(Default::default(), Default::default());
    engine.fire_raf(16.0);
    engine.eval("assert(!b.matches(':hover') && !root.matches(':hover'), 'detached target retained hover');")?;
    Ok(())
}
