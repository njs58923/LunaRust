use js_runtime::{
    components::{parse_events, parse_json, Channel, ComponentPort, MAX_MESSAGE},
    Engine,
};
use std::sync::Arc;
fn engine(port: Arc<ComponentPort>) -> Engine {
    let mut e = Engine::new();
    e.configure_component_port(port);
    e.update_tag_snapshot([(0, "space".into()), (1, "include".into())].into());
    e.update_hierarchy_snapshot([(0, -1), (1, 0)].into(), [(0, vec![1]), (1, vec![])].into());
    e.update_attr_snapshot(
        [
            (0, std::collections::HashMap::new()),
            (1, [("id".into(), "child".into())].into()),
        ]
        .into(),
    );
    e
}

#[test]
fn parent_messages_reach_child_once_with_frozen_data() {
    let parent = Arc::new(ComponentPort::default());
    let child = Arc::new(ComponentPort::default());
    let _channel = Channel::new(&parent, &child, 1, "https://child.test".into(),
        serde_json::json!({}), parse_events("").unwrap()).unwrap();
    {
        let mut p = engine(parent.clone());
        p.eval("globalThis.data={value:123};hiperspace.dimention.getElementById('child').send('talcosa',data);data.value=999;").unwrap();
    }
    assert!(child.take_wake());
    assert!(parent.drain().is_empty());
    let mut c = engine(child.clone());
    c.eval("globalThis.messages=[];component.addEventListener('message:talcosa', e=>messages.push(e));").unwrap();
    c.fire_raf(1.);
    c.eval("if(messages.length!==1||messages[0].detail.value!==123||!Object.isFrozen(messages[0].detail)||messages[0].target!==component||messages[0].isTrusted!==false||messages[0].bubbles!==false)throw Error('message delivery');").unwrap();
    c.fire_raf(2.);
    c.eval("if(messages.length!==1)throw Error('duplicate');").unwrap();
}

#[test]
fn parent_messages_are_bounded_isolated_and_cancelled_on_rebind() {
    let parent = Arc::new(ComponentPort::default());
    let child = Arc::new(ComponentPort::default());
    let c = Channel::new(&parent,&child,1,"https://child.test".into(),serde_json::json!({}),parse_events("change").unwrap()).unwrap();
    assert!(parent.send(2,"reset".into(),"{}").is_err());
    assert!(child.send(1,"reset".into(),"{}").is_err());
    assert!(parent.send(1,"*".into(),"{}").is_err());
    assert!(parent.send(1,"reset".into(),"{oops}").is_err());
    for i in 0..64 { parent.send(1,"reset".into(),&i.to_string()).unwrap(); }
    assert!(parent.send(1,"reset".into(),"65").is_err());
    let events=child.drain_messages();
    for (i,e) in events.iter().enumerate() { assert_eq!(e.detail,serde_json::json!(i)); assert_eq!(e.sequence,i as u64+1); }
    assert_eq!(events.len(),64);
    assert!(child.validate_message(&events[0].generation));
    let big=serde_json::to_string(&"a".repeat(MAX_MESSAGE-20)).unwrap();
    for _ in 0..8 { parent.send(1,"reset".into(),&big).unwrap(); child.emit("change".into(),&big).ok(); }
    assert!(parent.send(1,"reset".into(),&big).is_err());
    c.close();
    assert!(child.drain_messages().is_empty());
    assert!(!child.validate_message(&events[0].generation));
    parent.remove_child(1,&c.generation);
    let next=Arc::new(ComponentPort::default());
    let _next=Channel::new(&parent,&next,1,"https://next.test".into(),serde_json::json!({}),parse_events("").unwrap()).unwrap();
    // Closing frees both directions' shared byte budget.
    parent.send(1,"reset".into(),&big).unwrap();
    assert!(child.drain_messages().is_empty());
    assert_eq!(next.drain_messages().len(),1);
}
#[test]
fn component_initial_props_updates_events_and_disconnect() {
    let parent = Arc::new(ComponentPort::default());
    let child = Arc::new(ComponentPort::default());
    let channel = Channel::new(
        &parent,
        &child,
        1,
        "https://components.test".into(),
        parse_json(r#"{"volume":0.3,"title":"A"}"#, true).unwrap(),
        parse_events("change,ended").unwrap(),
    )
    .unwrap();
    // Initial state exists before user code. Only one V8 engine is entered at a time.
    {
        let mut c = engine(child.clone());
        c.eval(r#"if(!component.connected||component.props.volume!==0.3||!Object.isFrozen(component.props))throw Error('initial props');globalThis.changes=0;component.addEventListener('propschange',e=>{if(e.detail.props.volume!==0.8||e.detail.previous.volume!==0.3)throw Error('change data');changes++;});"#).unwrap();
        channel.update_props(parse_json(r#"{"volume":0.5,"title":"A"}"#, true).unwrap());
        channel.update_props(parse_json(r#"{"volume":0.8,"title":"A"}"#, true).unwrap());
        assert!(child.take_wake());
        c.fire_raf(1.);
        c.eval("if(changes!==1||component.props.volume!==0.8)throw Error('coalescing');component.emit('change',{value:8});").unwrap();
        assert!(parent.take_wake());
    }
    {
        let mut p = engine(parent.clone());
        p.eval(r#"globalThis.received=0;hiperspace.dimention.getElementById('child').addEventListener('component:change',e=>{if(e.detail.value!==8||e.origin!=='https://components.test'||e.isTrusted!==false||e.bubbles!==false)throw Error('metadata');received++;});"#).unwrap();
        p.fire_raf(1.);
        p.eval("if(received!==1)throw Error('event delivery');")
            .unwrap();
        p.fire_raf(2.);
        p.eval("if(received!==1)throw Error('duplicate delivery');")
            .unwrap();
    }
    child.emit("ended".into(), "{}").unwrap();
    channel.close();
    assert!(parent.drain().is_empty());
    assert!(child.emit("change".into(), "{}").is_err());
    assert!(!child.context().connected);
}
#[test]
fn component_limits_order_and_generation_rebinding() {
    let parent = Arc::new(ComponentPort::default());
    let child = Arc::new(ComponentPort::default());
    let c = Channel::new(
        &parent,
        &child,
        1,
        "https://a.test".into(),
        serde_json::json!({}),
        parse_events("change").unwrap(),
    )
    .unwrap();
    assert!(child.emit("click".into(), "{}").is_err());
    for i in 0..64 {
        child.emit("change".into(), &i.to_string()).unwrap();
    }
    assert!(child.emit("change".into(), "65").is_err());
    let received = parent.drain();
    assert_eq!(received.len(), 64);
    for (i, event) in received.iter().enumerate() {
        assert_eq!(event.detail, serde_json::json!(i));
        assert_eq!(event.sequence, i as u64 + 1);
    }
    let old_generation = c.generation.clone();
    c.close();
    parent.remove_child(1, &old_generation);
    let next = Arc::new(ComponentPort::default());
    let new = Channel::new(
        &parent,
        &next,
        1,
        "https://b.test".into(),
        serde_json::json!({}),
        parse_events("change").unwrap(),
    )
    .unwrap();
    assert_ne!(new.generation, old_generation);
    assert!(!parent.validate(1, &old_generation));
    assert!(parent.validate(1, &new.generation));
    assert!(child.emit("change".into(), "{}").is_err());
    let big = serde_json::to_string(&"a".repeat(MAX_MESSAGE - 20)).unwrap();
    for _ in 0..16 {
        next.emit("change".into(), &big).ok();
    }
    assert!(next.emit("change".into(), &big).is_err());
    parent.close();
    assert!(!next.context().connected);
}
#[test]
fn component_json_is_data_not_code_and_props_are_objects() {
    assert!(parse_json("{foo:2}", true).is_err());
    assert!(parse_json("[]", true).is_err());
    assert!(parse_json("null", true).is_err());
    assert!(parse_events("*").is_err());
    assert!(parse_events("__luna_hover").is_err());
    let deep = format!("{}0{}", "[".repeat(34), "]".repeat(34));
    assert!(parse_json(&deep, false).is_err());
    let mut e = engine(Arc::new(ComponentPort::default()));
    e.eval(r#"
      const include=hiperspace.dimention.getElementById('child');
      let failures=0;for(const v of [{x:undefined},{x:()=>{}},{x:NaN},{x:1n},{get x(){throw Error('getter executed')}}, {x:new Date()}]){try{include.props=v}catch(e){failures++}}
      if(failures!==6)throw Error('non-data accepted');
      include.props={a:1};include.props={...include.props,b:2};if(include.props.a!==1||include.props.b!==2)throw Error('optimistic props');
      globalThis.denied=false;component.emit('change',{}).catch(()=>denied=true);
    "#).unwrap();
    e.fire_raf(1.);
    e.eval("if(!denied)throw Error('root can emit without a parent')")
        .unwrap();
    let updates = e.drain_attr_updates();
    assert_eq!(updates.len(), 2);
    assert!(updates[1].2.contains("\"b\":2"));
}

#[test]
fn component_instances_are_independent_and_closed_events_do_not_leak() {
    let parent = Arc::new(ComponentPort::default());
    let a = Arc::new(ComponentPort::default());
    let b = Arc::new(ComponentPort::default());
    let first = Channel::new(
        &parent,
        &a,
        1,
        "https://same.test".into(),
        serde_json::json!({"value":1}),
        parse_events("change").unwrap(),
    )
    .unwrap();
    let second = Channel::new(
        &parent,
        &b,
        2,
        "https://same.test".into(),
        serde_json::json!({"value":2}),
        parse_events("change").unwrap(),
    )
    .unwrap();
    first.update_props(serde_json::json!({"value":3}));
    assert_eq!(b.context().props["value"], 2);
    a.emit("change".into(), "1").unwrap();
    b.emit("change".into(), "2").unwrap();
    first.close();
    let events = parent.drain();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].node_id, 2);
    assert_eq!(events[0].generation, second.generation);
    assert!(!a.context().connected);
    assert!(b.context().connected);
    assert!(Channel::new(
        &parent,
        &a,
        2,
        "https://same.test".into(),
        serde_json::json!({}),
        parse_events("change").unwrap()
    )
    .is_err());
}

#[test]
fn unchanged_context_omits_props_until_update_disconnect_or_rebind() {
    let parent = Arc::new(ComponentPort::default());
    let child = Arc::new(ComponentPort::default());
    let disconnected = child.context_if_changed(None, 0).unwrap();
    assert!(!disconnected.connected);
    assert!(child.context_if_changed(Some(""), 0).is_none());
    let props = serde_json::json!({"nested": {"value": 1}});
    let first = Channel::new(&parent, &child, 1, "https://props.test".into(),
        props.clone(), parse_events("").unwrap()).unwrap();
    let initial = child.context_if_changed(Some(""), 0).unwrap();
    assert_eq!(initial.revision, 1);
    assert!(child.context_if_changed(Some(&initial.generation), initial.revision).is_none());
    first.update_props(props.clone());
    assert!(child.context_if_changed(Some(&initial.generation), initial.revision).is_none());
    first.update_props(serde_json::json!({"nested": {"value": 2}}));
    let updated = child.context_if_changed(Some(&initial.generation), initial.revision).unwrap();
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.props["nested"]["value"], 2);
    assert_eq!(initial.props["nested"]["value"], 1);
    parent.remove_child(1, &first.generation);
    assert!(!child.context_if_changed(Some(&updated.generation), updated.revision).unwrap().connected);
    assert!(child.context_if_changed(Some(""), 0).is_none());
    let second = Channel::new(&parent, &child, 1, "https://props.test".into(),
        props, parse_events("").unwrap()).unwrap();
    // A generation change must invalidate a matching revision and props value.
    let rebound = child.context_if_changed(Some(&initial.generation), initial.revision).unwrap();
    assert_eq!(rebound.revision, initial.revision);
    assert_ne!(rebound.generation, initial.generation);
    assert_eq!(rebound.generation, second.generation);
    child.close();
    assert!(!child.context_if_changed(Some(&rebound.generation), rebound.revision).unwrap().connected);
    assert!(child.context_if_changed(Some(""), 0).is_none());
}

#[test]
fn cached_component_getters_preserve_frozen_props_and_observer_lifecycle() {
    let parent = Arc::new(ComponentPort::default());
    let child = Arc::new(ComponentPort::default());
    let props = serde_json::json!({"nested": {"value": 1}, "items": [1, 2]});
    let first = Channel::new(&parent, &child, 1, "https://props.test".into(),
        props.clone(), parse_events("").unwrap()).unwrap();
    let mut e = engine(child);
    e.eval(r#"
        globalThis.firstProps = component.props;
        globalThis.events = [];
        for (const type of ['connect', 'disconnect', 'propschange']) {
            component.addEventListener(type, event => events.push(event));
        }
        if(component.props!==firstProps||!Object.isFrozen(firstProps.nested)||!Object.isFrozen(firstProps.items))throw Error('props identity/freeze');
        firstProps.nested.value=99;
        if(component.props.nested.value!==1)throw Error('mutation escaped freeze');
    "#).unwrap();
    first.update_props(props);
    e.fire_raf(150.);
    e.eval("if(events.length||component.props!==firstProps)throw Error('equal props changed');").unwrap();
    first.update_props(serde_json::json!({"nested": {"value": 2}, "items": [1, 2]}));
    // An eager getter refreshes the cache but must not consume the observer event.
    e.eval("globalThis.updatedProps=component.props;if(updatedProps===firstProps||updatedProps.nested.value!==2||firstProps.nested.value!==1)throw Error('snapshot isolation');").unwrap();
    e.fire_raf(300.);
    e.eval(r#"
        if(events.length!==1||events[0].type!=='propschange'||events[0].detail.previous!==firstProps||events[0].detail.props!==updatedProps||events[0].detail.changedKeys.join(',')!=='nested')throw Error('getter consumed propschange');
        events.length=0;
    "#).unwrap();
    parent.remove_child(1, &first.generation);
    e.eval("if(component.connected||Object.keys(component.props).length)throw Error('closed cached context');").unwrap();
    e.fire_raf(450.);
    e.eval("if(events.length!==1||events[0].type!=='disconnect')throw Error('getter consumed disconnect');events.length=0;").unwrap();
    let child = Arc::new(ComponentPort::default());
    let _second = Channel::new(&parent, &child, 1, "https://props.test".into(),
        serde_json::json!({"nested": {"value": 1}, "items": [1, 2]}), parse_events("").unwrap()).unwrap();
    e.configure_component_port(child);
    e.eval("if(!component.connected||component.revision!==1||component.props.nested.value!==1)throw Error('rebound cache stale');").unwrap();
    e.fire_raf(600.);
    e.eval("if(events.length!==2||events[0].type!=='connect'||events[1].type!=='propschange')throw Error('rebind events missing');events.length=0;").unwrap();
    e.fire_raf(750.);
    e.eval("if(events.length)throw Error('duplicate observer events');").unwrap();
}

#[test]
#[ignore = "manual unchanged street-sized component props benchmark"]
fn benchmark_unchanged_component_context() {
    use std::time::Instant;
    let parent = Arc::new(ComponentPort::default());
    let child = Arc::new(ComponentPort::default());
    let props = serde_json::json!({"pieces": (0..128).map(|id| serde_json::json!({
        "id": id, "position": [1.25, 2.0, -1.5], "scale": [0.5, 2.0, 0.25],
        "rotation": [0, 90, 0], "material": "facade-stone"
    })).collect::<Vec<_>>()});
    let props_bytes = serde_json::to_vec(&props).unwrap().len();
    let _channel = Channel::new(&parent, &child, 1, "https://props.test".into(),
        props, parse_events("").unwrap()).unwrap();
    let mut e = engine(child);
    e.eval(r#"
        globalThis.legacyRead = (() => {
            let cached=null;
            function freeze(v){if(v&&typeof v==='object'){for(const x of Object.values(v))freeze(x);Object.freeze(v);}return v;}
            return () => {
                const s=Deno.core.ops.op_component_context();
                if(!cached||cached.generation!==s.generation||cached.revision!==s.revision||cached.connected!==s.connected)cached=freeze(s);
                return cached.props;
            };
        })();
        globalThis.conditionalRead=()=>component.props;
        legacyRead();conditionalRead();
        globalThis.readRepeatedly=(read,count)=>{for(let i=0;i<count;i++){if(read().pieces.length!==128)throw Error('props changed');}};
    "#).unwrap();
    let mut legacy = Vec::new();
    let mut conditional = Vec::new();
    const POLLS: usize = 2500;
    for sample in 0..9 {
        for optimized in if sample % 2 == 0 { [false, true] } else { [true, false] } {
            let source = format!("readRepeatedly({}, {POLLS});", if optimized { "conditionalRead" } else { "legacyRead" });
            let started = Instant::now();
            e.eval(&source).unwrap();
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            if optimized { conditional.push(elapsed); } else { legacy.push(elapsed); }
        }
    }
    legacy.sort_by(f64::total_cmp);
    conditional.sort_by(f64::total_cmp);
    // Normalize CPU time to one 25-facade service pass; no wall-clock sleeps or FPS claim.
    eprintln!("component_context props_bytes={props_bytes} polls={POLLS} service_interval_ms=150 legacy_25_polls_ms={:.6} conditional_25_polls_ms={:.6}",
        legacy[4] * 25.0 / POLLS as f64, conditional[4] * 25.0 / POLLS as f64);
}
