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
