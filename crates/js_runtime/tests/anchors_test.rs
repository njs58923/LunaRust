use js_runtime::{
    anchors::{Reply, Source},
    Engine,
};

#[test]
fn native_aliases_and_elements_register_once_and_errors_reach_the_caller() {
    let mut engine = Engine::new();
    engine
        .eval(
            r#"
      globalThis.status = [];
      dimension.anchors.create('right-grip', '__vr_right_grip_root')
        .then(handle => { globalThis.handle = handle; status.push('ready'); });
      dimension.anchors.create('denied', '__private').catch(e => status.push(e.message));
    "#,
        )
        .unwrap();
    let requests = engine.drain_anchor_requests();
    assert_eq!(requests.len(), 2);
    assert!(
        matches!(&requests[0].source, Some(Source::Native(name)) if name == "__vr_right_grip_root")
    );
    engine.push_anchor_replies(vec![
        Reply {
            id: requests[0].id,
            error: None,
        },
        Reply {
            id: requests[1].id,
            error: Some("Unknown native reference".into()),
        },
    ]);
    engine.fire_raf(16.);
    engine
        .eval("if(status.join('|')!=='ready|Unknown native reference')throw Error(status);")
        .unwrap();
    for t in [32., 48., 64.] {
        engine.fire_raf(t);
        assert!(engine.drain_anchor_requests().is_empty());
    }
    engine
        .eval("handle.dispose(); dimension.anchors.create('node', new HSMLElement(7));")
        .unwrap();
    let requests = engine.drain_anchor_requests();
    assert!(requests[0].source.is_none());
    assert!(matches!(requests[1].source, Some(Source::Element(7))));
}

#[test]
fn native_strings_are_reserved_and_request_growth_is_bounded() {
    let mut engine = Engine::new();
    engine.eval(r#"
      globalThis.errors = [];
      dimension.anchors.create('a', '#dom-id').catch(e => errors.push(e.message));
      dimension.anchors.create('invalid key', '__native').catch(e => errors.push(e.message));
      for(let i=0;i<80;i++)dimension.anchors.create('key'+i,'__native').catch(e=>errors.push(e.message));
    "#).unwrap();
    assert_eq!(engine.drain_anchor_requests().len(), 64);
    engine
        .eval("if(errors.length!==18)throw Error('unbounded or invalid API:'+errors.length);")
        .unwrap();
}
