//! Load-time authoring diagnostics. Never runs in the transform/render frame loop.
use quick_xml::{events::Event, Reader};
use std::collections::{HashMap, HashSet};

pub fn document_warnings(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    let mut warnings = Vec::new();
    let mut ids = HashSet::new();
    let mut scope = 0u64;
    let mut next_scope = 0u64;
    let mut stack = Vec::new();
    loop {
        match reader.read_event() {
            Ok(event @ (Event::Start(_) | Event::Empty(_))) => {
                let empty = matches!(&event, Event::Empty(_));
                let e = match event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                let previous = scope;
                if e.name().as_ref() == b"space" {
                    next_scope += 1;
                    scope = next_scope;
                }
                let attrs: HashMap<_, _> = e
                    .attributes()
                    .filter_map(Result::ok)
                    .filter_map(|a| {
                        a.unescape_value().ok().map(|v| {
                            (
                                String::from_utf8_lossy(a.key.as_ref()).into_owned(),
                                v.into_owned(),
                            )
                        })
                    })
                    .collect();
                if let Some(id) = attrs.get("id").filter(|s| !s.is_empty()) {
                    if !ids.insert((scope, id.clone())) {
                        warnings.push(format!("Duplicate id '{id}' in the same document space"));
                    }
                }
                if e.name().as_ref() == b"meta"
                    && attrs
                        .get("type")
                        .is_some_and(|t| matches!(t.as_str(), "position" | "rotation" | "scale"))
                {
                    warnings.push(format!(
                        "Ignored <meta type='{}'>: put transforms on a group or mounted space",
                        attrs["type"]
                    ));
                }
                if empty {
                    scope = previous;
                } else {
                    stack.push(previous);
                }
            }
            Ok(Event::End(_)) => {
                scope = stack.pop().unwrap_or(0);
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        if warnings.len() >= 64 {
            warnings.push("Further document diagnostics suppressed".into());
            break;
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scopes_ids_and_ignores_script_text() {
        let warnings = document_warnings(
            r#"<hsml><meta type="position"/><space><box id="a"/><box id="a"/><space><box id="a"/></space><script><![CDATA[<box id="a"/>]]></script></space></hsml>"#,
        );
        assert_eq!(warnings.len(), 2);
        assert!(warnings[1].contains("Duplicate"));
        assert!(document_warnings(
            "<hsml><space><box id='a'/></space><space><box id='a'/></space></hsml>"
        )
        .is_empty());
    }
    #[test]
    fn unsupported_fetch_options_reject_without_network_request() {
        let mut engine = js_runtime::Engine::new();
        engine
            .eval("fetch('/api', {cache:'no-store'}).catch(e => console.log(e.message));")
            .unwrap();
        engine.fire_raf(0.0);
        assert!(engine.drain_fetch_queue().is_empty());
        assert!(engine
            .drain_logs()
            .iter()
            .any(|(_, m)| m.contains("Unsupported fetch option: cache")));
    }
}
