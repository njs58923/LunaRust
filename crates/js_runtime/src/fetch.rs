use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FetchRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    #[serde(default)]
    pub body_bytes: Option<Vec<u8>>,
    pub redirect: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FetchResponse {
    pub url: String,
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
    pub redirected: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn javascript_fetch_preserves_request_and_http_error_response() {
        let mut engine = crate::Engine::new();
        engine.configure_document_location("https://portals.test/components/door?uuid=one".into());
        engine.eval(r#"
            globalThis.done=false; globalThis.failure='';
            const h = new Headers({'Content-Type':'application/json'});
            h.set('Authorization','Bearer secret');
            fetch('../api', {method:'POST', headers:h, body:JSON.stringify({title:'máquina'})})
              .then(async r => {
                if(r.ok || r.status!==409 || r.statusText!=='Conflict' || r.headers.get('X-Revision')!=='2') throw Error('metadata');
                const copy=r.clone();
                if ((await r.json()).error!=='exists' || !r.bodyUsed) throw Error('body');
                let twice=false; try { await r.text(); } catch(e) { twice=e instanceof TypeError; }
                if(!twice || await copy.text()!=='{"error":"exists"}') throw Error('consumption');
                let immutable=false; try { r.headers.set('x','y'); } catch(e) { immutable=e instanceof TypeError; }
                if(!immutable) throw Error('headers guard');
                done=true;
              }).catch(e=>failure=String(e));
        "#).unwrap();
        let (id, request) = engine.drain_fetch_queue().pop().unwrap();
        assert_eq!(request.url, "https://portals.test/api");
        assert_eq!(request.method, "POST");
        assert_eq!(request.body.as_deref(), Some("{\"title\":\"máquina\"}"));
        assert!(request
            .headers
            .contains(&("authorization".into(), "Bearer secret".into())));
        engine.push_fetch_result(
            id,
            Ok(FetchResponse {
                status: 409,
                status_text: "Conflict".into(),
                url: request.url,
                headers: vec![("X-Revision".into(), "2".into())],
                body: b"{\"error\":\"exists\"}".to_vec(),
                redirected: false,
            }),
        );
        for tick in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            engine.fire_raf(tick as f64 * 10.0);
            engine.eval("if (failure) throw Error(failure);").unwrap();
            if engine.eval("if (!done) throw Error('pending');").is_ok() { return; }
        }
        panic!("fetch promise did not settle");
    }

    #[test]
    fn form_body_and_invalid_options_and_network_errors() {
        let mut engine = crate::Engine::new();
        engine.configure_document_location("https://portals.test/door".into());
        engine
            .eval(
                r#"
            globalThis.errors=0;
            fetch('/api',{method:'GET',body:'x'}).catch(()=>errors++);
            fetch('/api',{method:'TRACE'}).catch(()=>errors++);
            fetch('/api',{headers:{x:'a\r\nb'}}).catch(()=>errors++);
            fetch('/api',{body:{a:1},method:'POST'}).catch(()=>errors++);
            fetch('/api',{signal:{}}).catch(()=>errors++);
            globalThis.networkError=false;
            fetch('/api',{method:'PATCH',body:new URLSearchParams({q:'a b'})})
              .catch(e=>networkError=e instanceof TypeError);
        "#,
            )
            .unwrap();
        let requests = engine.drain_fetch_queue();
        assert_eq!(requests.len(), 1);
        let (id, request) = &requests[0];
        assert_eq!(request.body.as_deref(), Some("q=a+b"));
        assert!(request.headers.contains(&(
            "content-type".into(),
            "application/x-www-form-urlencoded;charset=UTF-8".into()
        )));
        engine.push_fetch_result(*id, Err("Network unavailable".into()));
        std::thread::sleep(std::time::Duration::from_millis(20));
        engine.fire_raf(30.0);
        engine
            .eval("if(errors!==5 || !networkError) throw Error('validation/rejection');")
            .unwrap();
    }
}
