use js_runtime::Engine;
use serde_json::json;

#[test]
fn keyboard_native_mailbox_focus_and_cancelable_events() {
    let mut engine = Engine::new();
    engine.eval(r#"
      globalThis.received=[];
      globalThis.field={dispatchEvent(e){received.push(e.type);if(e.type==='keydown')e.preventDefault();},insertText(){throw Error('canceled key inserted');}};
      keyboard.focus(field,{editable:true});
    "#).unwrap();
    let commands = engine.drain_keyboard_commands();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["action"], "focus");
    engine.push_keyboard_events(vec![
        json!({"type":"input","packet":{"type":"keydown","key":"a","code":"KeyA","text":"a"}}),
    ]);
    engine.eval("__luna_keyboard_pump()").unwrap();
    engine.push_keyboard_events(vec![json!({"type":"state","focused":false})]);
    engine.eval("__luna_keyboard_pump();if(received.join(',')!=='focus,keydown,keyup,blur')throw Error(received);if(keyboard.activeElement!==null)throw Error('stale focus');").unwrap();
}
