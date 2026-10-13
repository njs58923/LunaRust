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

#[test]
fn program_definitions_use_the_same_registration_and_error_contract() {
    let mut engine = Engine::new();
    engine.eval("if(!dimension.anchors.supportsPrograms)throw Error('feature flag');globalThis.error=null;dimension.anchors.create('computed',{type:'program',version:1,inputs:{a:'right-grip'},code:'input pose a\\noutput pose result = a'}).catch(e=>error=e.message);").unwrap();
    let requests = engine.drain_anchor_requests();
    assert!(matches!(&requests[0].source,Some(Source::Program(v)) if v["version"]==1));
    engine.push_anchor_replies(vec![Reply {
        id: requests[0].id,
        error: Some("line 3: Unknown instruction".into()),
    }]);
    engine.fire_raf(16.);
    engine
        .eval(
            "if(error!=='line 3: Unknown instruction')throw Error('missing compiler diagnostic');",
        )
        .unwrap();
}

#[test]
fn input_history_is_ordered_bounded_and_independent_of_raf() {
    use js_runtime::anchors::{AnchorPort, CapturedFrame, Sample, HISTORY_FRAMES};
    use std::{sync::Arc, time::Instant};
    let mut engine = Engine::new();
    let port = Arc::new(AnchorPort::default());
    engine.configure_anchor_port(port.clone());
    engine.eval(r#"
      globalThis.frames=[]; globalThis.raf=false;
      globalThis.listener=dimension.anchors.listen(['left-grip','right-grip'], frame=>frames.push(frame), {priority:'input',delivery:'history'});
      requestAnimationFrame(()=>raf=true);
    "#).unwrap();
    let id = port.subscriptions()[0].id;
    for frame_id in 0..70 {
        port.publish(
            id,
            CapturedFrame {
                frame_id,
                captured: Instant::now(),
                discontinuous: false,
                samples: (0..2)
                    .map(|index| Sample {
                        index,
                        available: true,
                        position: [frame_id as f32, index as f32, 0.],
                        rotation: [0., 0., 0., 1.],
                        scale: [1.; 3],
                        error: None,
                        input: None,
                    })
                    .collect(),
            },
        );
    }
    engine.pump_anchor_input().unwrap();
    engine.eval(&format!(r#"
      if(raf || frames.length!=={HISTORY_FRAMES} || frames[0].frameId!==6 || !frames[0].discontinuous)throw Error('history or priority');
      for(let i=0;i<frames.length;i++){{
        const f=frames[i];
        if(f.samples.length!==2 || f.samples[1].position.x!==f.frameId || f.samples[0].target!=='left-grip')throw Error('incoherent frame');
        if(f.timestamp>performance.now()+1 || (i&&f.timestamp<frames[i-1].timestamp))throw Error('clock');
      }}
      listener.dispose();
    "#)).unwrap();
    assert!(port.subscriptions().is_empty());
    engine.fire_raf(100.);
    engine.eval("if(!raf)throw Error('RAF lost');").unwrap();
}

#[test]
fn latest_and_normal_priority_coalesce_until_raf_and_listener_growth_is_bounded() {
    use js_runtime::anchors::{AnchorPort, CapturedFrame};
    use std::{sync::Arc, time::Instant};
    let mut engine = Engine::new();
    let port = Arc::new(AnchorPort::default());
    engine.configure_anchor_port(port.clone());
    engine.eval(r#"
      globalThis.frames=[];
      dimension.anchors.listen('right-grip', f=>frames.push(f));
      for(let i=1;i<16;i++)dimension.anchors.listen('right-grip',()=>{});
      try { dimension.anchors.listen('right-grip',()=>{}); throw 'unbounded'; } catch(e) { if(e==='unbounded')throw e; }
    "#).unwrap();
    let id = port.subscriptions().iter().map(|s| s.id).min().unwrap();
    for frame_id in 0..4 {
        port.publish(
            id,
            CapturedFrame {
                frame_id,
                captured: Instant::now(),
                discontinuous: false,
                samples: vec![],
            },
        );
    }
    engine.pump_anchor_input().unwrap();
    engine
        .eval("if(frames.length)throw Error('normal fired early');")
        .unwrap();
    engine.fire_raf(16.);
    engine
        .eval("if(frames.length!==1||frames[0].frameId!==3)throw Error('latest');")
        .unwrap();
}
