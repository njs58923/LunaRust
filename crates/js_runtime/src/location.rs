use deno_core::{op2, OpState};
use serde_json::json;

pub(crate) struct DocumentLocation(pub String);

#[op2]
#[string]
pub(crate) fn op_document_location(state: &mut OpState) -> String {
    state.borrow::<DocumentLocation>().0.clone()
}

#[op2]
#[serde]
pub(crate) fn op_url_parse(
    #[string] input: &str,
    #[string] base: &str,
    #[string] part: &str,
    #[string] value: &str,
) -> serde_json::Value {
    let parsed = if base.is_empty() {
        url::Url::parse(input)
    } else {
        url::Url::parse(base).and_then(|base| base.join(input))
    };
    let Ok(mut url) = parsed else {
        return json!({"error":"Invalid URL"});
    };
    let valid = match part {
        "" => true,
        "protocol" => url.set_scheme(value.trim_end_matches(':')).is_ok(),
        "hostname" => url.set_host(Some(value)).is_ok(),
        "port" => {
            if value.is_empty() {
                url.set_port(None).is_ok()
            } else {
                value
                    .parse::<u16>()
                    .ok()
                    .is_some_and(|port| url.set_port(Some(port)).is_ok())
            }
        }
        "host" => {
            // Use the URL parser for IPv6 literals and optional ports.
            match url::Url::parse(&format!("{}://{value}/", url.scheme())) {
                Ok(host)
                    if host.username().is_empty()
                        && host.password().is_none()
                        && host.path() == "/" =>
                {
                    url.set_host(host.host_str()).is_ok() && url.set_port(host.port()).is_ok()
                }
                _ => false,
            }
        }
        "pathname" => {
            url.set_path(value);
            true
        }
        "search" => {
            url.set_query(if value.is_empty() {
                None
            } else {
                Some(value.strip_prefix('?').unwrap_or(value))
            });
            true
        }
        "hash" => {
            url.set_fragment(if value.is_empty() {
                None
            } else {
                Some(value.strip_prefix('#').unwrap_or(value))
            });
            true
        }
        "username" => url.set_username(value).is_ok(),
        "password" => url.set_password(Some(value)).is_ok(),
        _ => false,
    };
    if !valid {
        return json!({"error":"Invalid URL component"});
    }
    let hostname = url.host().map(|host| host.to_string()).unwrap_or_default();
    let port = url.port().map(|port| port.to_string()).unwrap_or_default();
    json!({
        "href":url.as_str(), "origin":url.origin().ascii_serialization(),
        "protocol":format!("{}:",url.scheme()),
        "host":if port.is_empty() {hostname.clone()} else {format!("{hostname}:{port}")},
        "hostname":hostname,"port":port,"pathname":url.path(),
        "search":url.query().filter(|q| !q.is_empty()).map(|q|format!("?{q}")).unwrap_or_default(),
        "hash":url.fragment().filter(|q| !q.is_empty()).map(|q|format!("#{q}")).unwrap_or_default(),
        "username":url.username(),"password":url.password().unwrap_or_default()
    })
}

#[op2]
#[serde]
pub(crate) fn op_query_parse(#[string] query: &str) -> Vec<(String, String)> {
    url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect()
}

#[op2]
#[string]
pub(crate) fn op_query_encode(#[serde] pairs: Vec<(String, String)>) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

#[cfg(test)]
mod tests {
    use crate::Engine;

    #[test]
    fn document_location_and_relative_navigation_keep_committed_identity() {
        let mut engine = Engine::new();
        let url =
            "https://portals.test:8443/components/door.hsml?uuid=abc&title=La+máquina#llegada";
        engine.configure_document_location(url.into());
        engine.eval(r#"
            function check(ok, msg) { if (!ok) throw new Error(msg); }
            check(location === hiperspace.location && window.location === location, 'identity');
            check(location.protocol === 'https:' && location.host === 'portals.test:8443', 'host');
            check(location.hostname === 'portals.test' && location.port === '8443', 'port');
            check(location.pathname === '/components/door.hsml', 'document path');
            check(location.hash === '#llegada' && location.origin === 'https://portals.test:8443', 'fragment/origin');
            const params = new URLSearchParams(location.search);
            check(params.get('uuid') === 'abc' && params.get('title') === 'La máquina', 'query');
            const committed = location.href;
            location.assign('../world.hsml#sur');
            check(location.href === committed, 'request is not commit');
            globalThis._lunaCurrentUrl = 'https://forged.test/';
            hiperspace.setHiperSpace({location:'https://forged.test/'}, null);
            check(location.href === committed, 'JS cannot set document identity');
            location.hash = 'otro';
            location.search = '?uuid=next';
            location.replace('/replaced');
            location.reload();
            window.location = '/assigned';
        "#).unwrap();
        assert_eq!(engine.drain_navigate_queue(), vec![
            "https://portals.test:8443/world.hsml#sur".to_string(),
            "https://portals.test:8443/components/door.hsml?uuid=abc&title=La+m%C3%A1quina#otro".into(),
            "https://portals.test:8443/components/door.hsml?uuid=next#llegada".into(),
            "https://portals.test:8443/replaced".into(),
            "https://portals.test:8443/components/door.hsml?uuid=abc&title=La+m%C3%A1quina#llegada".into(),
            "https://portals.test:8443/assigned".into(),
        ]);
        engine.configure_document_location("https://other.test/new?x=2".into());
        engine
            .eval("check(location.href === 'https://other.test/new?x=2', 'host commit');")
            .unwrap();
    }

    #[test]
    fn url_and_search_params_support_live_updates_and_encoding() {
        let mut engine = Engine::new();
        engine.eval(r#"
            function check(ok,msg) { if (!ok) throw new Error(msg); }
            const url = new URL('../door?x=1&x=2&empty=#sur', 'https://EXAMPLE.test:443/a/b');
            check(url.href === 'https://example.test/door?x=1&x=2&empty=#sur', 'resolution');
            const p = url.searchParams;
            check(p.getAll('x').join(',') === '1,2' && p.get('missing') === null, 'duplicates');
            check(p.get('empty') === '' && p.size === 3, 'empty');
            p.set('x','3'); p.append('title','árbol & sol');
            check(url.search === '?x=3&empty=&title=%C3%A1rbol+%26+sol', 'live query');
            url.search = '?b=2&a=1&a=3';
            check(p === url.searchParams && p.get('b') === '2', 'stable object');
            p.sort(); check(p.toString() === 'a=1&a=3&b=2', 'stable sort');
            p.delete('a','1'); check(p.has('a','3') && !p.has('a','1'), 'value filters');
            const items = []; p.forEach((v,k,self) => { check(self === p, 'callback'); items.push([k,v]); });
            check(JSON.stringify(items) === JSON.stringify([...p]), 'iteration');
            url.href = 'https://[::1]:8443/new?z=4';
            check(url.hostname === '[::1]' && url.port === '8443' && p.get('z') === '4', 'IPv6');
            const malformed = new URLSearchParams('bad=%ZZ&utf=%FF&plus=a+b&literal=%2B');
            check(malformed.get('bad') === '%ZZ' && malformed.get('utf') === '\uFFFD', 'tolerant decoding');
            check(malformed.get('plus') === 'a b' && malformed.get('literal') === '+', 'plus');
            check(new URLSearchParams({a:'\ud800'}).toString() === 'a=%EF%BF%BD', 'USV string');
            check(new URLSearchParams([['a',1],['a',2]]).getAll('a').join(',') === '1,2', 'sequence');
            check(!URL.canParse('http://[') && URL.canParse('/x','https://a.test'), 'validation');
            let invalid=false; try { new URL('http://['); } catch(e) { invalid=e instanceof TypeError; }
            check(invalid, 'TypeError');
        "#).unwrap();
    }

    #[test]
    fn native_location_and_distinct_isolates() {
        let mut a = Engine::new();
        let mut b = Engine::new();
        a.configure_document_location("luna://home?theme=green#entrada".into());
        b.configure_document_location("https://external.test/door?uuid=two".into());
        a.eval("if (location.protocol !== 'luna:' || new URLSearchParams(location.search).get('theme') !== 'green') throw Error('native');").unwrap();
        b.eval("if (location.host !== 'external.test' || new URLSearchParams(location.search).get('uuid') !== 'two') throw Error('isolation');").unwrap();
    }
}
