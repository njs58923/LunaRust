use virtual_dom::{parse_xml, serialize_xml};
use virtual_dom::dom::element::{build_world, Attrs, Hierarchy, Tag, Transform2};
use specs::WorldExt;

fn make_world() -> specs::World {
    build_world()
}

#[test]
fn parse_mixed_scripts_keeps_all_boundaries_and_attributes() {
    use specs::Join;
    use virtual_dom::dom::hsml::Script;
    let mut world = make_world();
    let xml = r#"<hsml><space><script src="before.js"/>
      <script><![CDATA[const before = '<box/>';]]></script>
      <script>const raw = 1 &lt; 2 &amp;&amp; 3 > 2;</script>
      <box id="kept" x="3" sx="2" title="A &amp; B"/>
      <script><![CDATA[const after = ']]><![CDATA[>';]]></script>
      <script>const second = ']]>';</script>
      <script src="after.js"/></space></hsml>"#;
    parse_xml(&mut world, xml).unwrap();
    let scripts = world.read_storage::<Script>();
    let scripts: Vec<_> = (&scripts).join().collect();
    assert_eq!(scripts.len(), 6);
    assert_eq!(scripts[0].src.as_deref(), Some("before.js"));
    assert_eq!(scripts[1].inline.as_deref(), Some("const before = '<box/>';"));
    assert_eq!(scripts[2].inline.as_deref(), Some("const raw = 1 < 2 && 3 > 2;"));
    assert_eq!(scripts[3].inline.as_deref(), Some("const after = '>';"));
    assert_eq!(scripts[4].inline.as_deref(), Some("const second = ']]>';"));
    assert_eq!(scripts[5].src.as_deref(), Some("after.js"));
    let attrs = world.read_storage::<Attrs>();
    let transforms = world.read_storage::<Transform2>();
    let (attrs, transform) = (&attrs, &transforms).join()
        .find(|(attrs, _)| attrs.0.get("id").is_some_and(|id| id == "kept")).unwrap();
    assert_eq!(attrs.0["title"], "A & B");
    assert_eq!((transform.position.x, transform.scale.x, transform.scale.y), (3.0, 2.0, 1.0));
}

#[test]
#[ignore = "manual CPU parser benchmark; no rendering or network"]
fn benchmark_static_document_parse() {
    use std::time::Instant;
    for count in [1_000, 10_000] {
        let mut xml = String::from("<hsml><space><script src='facade.js'/>");
        for i in 0..count {
            use std::fmt::Write;
            write!(&mut xml, "<model id='mesh_{i}' src='mesh://1/{i}' x='{i}' y='1' material-unlit='true' touchable='false'/>").unwrap();
        }
        xml.push_str("</space></hsml>");
        let mut samples = Vec::new();
        for sample in 0..24 {
            let mut world = make_world();
            let start = Instant::now();
            let root = parse_xml(&mut world, &xml).unwrap();
            if sample >= 4 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
            let hierarchy = world.read_storage::<Hierarchy>();
            let space = hierarchy.get(root).unwrap().children[0];
            assert_eq!(hierarchy.get(space).unwrap().children.len(), count + 1);
        }
        samples.sort_by(f64::total_cmp);
        println!("static_parse models={count} bytes={} median_ms={:.4} p95_ms={:.4}", xml.len(), samples[10], samples[19]);
    }
}

// ─── parse_xml ───────────────────────────────────────────────────────────────

#[test]
fn parse_minimal_hsml() {
    let mut world = make_world();
    let xml = r#"<hsml></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    let tags = world.read_storage::<Tag>();
    let tag = tags.get(root).expect("no Tag on root");
    assert_eq!(tag.0, "hsml");
}

#[test]
fn parse_space_with_children() {
    let mut world = make_world();
    let xml = r#"<hsml><space><box x="1" y="0" z="0"/></space></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    let hier = world.read_storage::<Hierarchy>();
    let root_hier = hier.get(root).expect("no hierarchy on root");
    assert_eq!(root_hier.children.len(), 1);

    let space = root_hier.children[0];
    let tags = world.read_storage::<Tag>();
    assert_eq!(tags.get(space).unwrap().0, "space");

    let space_hier = hier.get(space).unwrap();
    assert_eq!(space_hier.children.len(), 1);
}

#[test]
fn parse_text_element_has_tag() {
    let mut world = make_world();
    let xml = r#"<hsml><space><text value="Hello" size="0.2"/></space></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    let hier = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();

    let space = hier.get(root).unwrap().children[0];
    let text_ent = hier.get(space).unwrap().children[0];
    assert_eq!(tags.get(text_ent).unwrap().0, "text");
}

#[test]
fn parse_attributes_stored_in_attrs() {
    let mut world = make_world();
    let xml = r##"<hsml><box color="#FF0000" x="1"/></hsml>"##;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    let hier = world.read_storage::<Hierarchy>();
    let attrs = world.read_storage::<Attrs>();

    let box_ent = hier.get(root).unwrap().children[0];
    let a = attrs.get(box_ent).expect("no Attrs on box");
    assert_eq!(a.0.get("color").map(|s| s.as_str()), Some("#FF0000"));
    assert_eq!(a.0.get("x").map(|s| s.as_str()), Some("1"));
}

#[test]
fn parse_transform_position_from_xyz() {
    let mut world = make_world();
    let xml = r#"<hsml><box x="2" y="3" z="4"/></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    let hier = world.read_storage::<Hierarchy>();
    let transforms = world.read_storage::<Transform2>();

    let box_ent = hier.get(root).unwrap().children[0];
    let tr = transforms.get(box_ent).expect("no Transform2 on box");
    assert!((tr.position.x - 2.0).abs() < 0.001);
    assert!((tr.position.y - 3.0).abs() < 0.001);
    assert!((tr.position.z - 4.0).abs() < 0.001);
}

#[test]
fn parse_transform_scale_uniform() {
    let mut world = make_world();
    let xml = r#"<hsml><box s="2"/></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    let hier = world.read_storage::<Hierarchy>();
    let transforms = world.read_storage::<Transform2>();

    let box_ent = hier.get(root).unwrap().children[0];
    let tr = transforms.get(box_ent).expect("no Transform2");
    assert!((tr.scale.x - 2.0).abs() < 0.001);
    assert!((tr.scale.y - 2.0).abs() < 0.001);
    assert!((tr.scale.z - 2.0).abs() < 0.001);
}

#[test]
fn parse_default_scale_is_one() {
    let mut world = make_world();
    let xml = r#"<hsml><box/></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    let hier = world.read_storage::<Hierarchy>();
    let transforms = world.read_storage::<Transform2>();
    let box_ent = hier.get(root).unwrap().children[0];
    let tr = transforms.get(box_ent).expect("no Transform2");
    assert!((tr.scale.x - 1.0).abs() < 0.001);
    assert!((tr.scale.y - 1.0).abs() < 0.001);
    assert!((tr.scale.z - 1.0).abs() < 0.001);
}

#[test]
fn parse_model_src_attribute() {
    use virtual_dom::dom::hsml::Model;
    let mut world = make_world();
    let xml = r#"<hsml><model src="tree.glb"/></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    let hier = world.read_storage::<Hierarchy>();
    let models = world.read_storage::<Model>();
    let model_ent = hier.get(root).unwrap().children[0];
    let m = models.get(model_ent).expect("no Model component");
    assert_eq!(m.src.as_deref(), Some("tree.glb"));
}

#[test]
fn parse_script_src_attribute() {
    use virtual_dom::dom::hsml::Script;
    let mut world = make_world();
    let xml = r#"<hsml><script src="luna://internal/home_navigation.js"/></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    let hier = world.read_storage::<Hierarchy>();
    let scripts = world.read_storage::<Script>();
    let script_ent = hier.get(root).unwrap().children[0];
    let s = scripts.get(script_ent).expect("no Script component");
    assert_eq!(s.src.as_deref(), Some("luna://internal/home_navigation.js"));
}

#[test]
fn parse_inline_script() {
    use virtual_dom::dom::hsml::Script;
    let mut world = make_world();
    let xml = r#"<hsml><script>console.log('hello');</script></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    let hier = world.read_storage::<Hierarchy>();
    let scripts = world.read_storage::<Script>();
    let script_ent = hier.get(root).unwrap().children[0];
    let s = scripts.get(script_ent).expect("no Script component");
    assert!(s.src.is_none(), "inline script should have no src");
    assert_eq!(s.inline.as_deref(), Some("console.log('hello');"));
}

#[test]
fn parse_script_with_src_has_no_inline() {
    use virtual_dom::dom::hsml::Script;
    let mut world = make_world();
    let xml = r#"<hsml><script src="luna://test.js"></script></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    let hier = world.read_storage::<Hierarchy>();
    let scripts = world.read_storage::<Script>();
    let script_ent = hier.get(root).unwrap().children[0];
    let s = scripts.get(script_ent).expect("no Script component");
    assert_eq!(s.src.as_deref(), Some("luna://test.js"));
    assert!(s.inline.is_none(), "src-based script should have no inline");
}

#[test]
fn parse_inline_script_with_arrow_functions() {
    use virtual_dom::dom::hsml::Script;
    let mut world = make_world();
    let xml = r#"<hsml><space><script>
    const btn = hiperspace.dimention.getElementById('btn');
    if (btn) btn.addEventListener('click', () => { location.href = 'luna://home'; });
    console.log('[test] loaded');
    </script></space></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    let hier = world.read_storage::<Hierarchy>();
    let scripts = world.read_storage::<Script>();
    // root -> space -> script
    let space = hier.get(root).unwrap().children[0];
    let script_ent = hier.get(space).unwrap().children[0];
    let s = scripts.get(script_ent).expect("no Script component");
    assert!(s.src.is_none());
    let code = s.inline.as_ref().expect("should have inline code");
    assert!(code.contains("getElementById"));
    assert!(code.contains("luna://home"));
    assert!(code.contains("=>"));
}

#[test]
fn parse_inline_script_with_xml_entities() {
    use virtual_dom::dom::hsml::Script;
    let mut world = make_world();
    let xml = r#"<hsml><space><script>
    for (var i = 0; i &lt; 10; i++) { console.log(i); }
    var x = true &amp;&amp; false;
    </script></space></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    let hier = world.read_storage::<Hierarchy>();
    let scripts = world.read_storage::<Script>();
    let space = hier.get(root).unwrap().children[0];
    let script_ent = hier.get(space).unwrap().children[0];
    let s = scripts.get(script_ent).expect("no Script component");
    let code = s.inline.as_ref().expect("should have inline code");
    // XML entities should be unescaped
    assert!(code.contains("i < 10"), "should unescape &lt; to <, got: {}", code);
    assert!(code.contains("&&"), "should unescape &amp;&amp; to &&, got: {}", code);
}

#[test]
fn parse_inline_script_accepts_raw_js_operators_without_xml_escaping() {
    use virtual_dom::dom::hsml::Script;

    let mut world = make_world();
    let xml = r#"
        <hsml>
            <space>
                <script>
                    for (let i = 0; i < 3; i++) {
                        if (i && true) console.log(i);
                    }
                </script>
            </space>
        </hsml>
    "#;

    let root = parse_xml(&mut world, xml).expect("parse failed");

    let hier = world.read_storage::<Hierarchy>();
    let scripts = world.read_storage::<Script>();
    let space = hier.get(root).unwrap().children[0];
    let script_ent = hier.get(space).unwrap().children[0];
    let s = scripts.get(script_ent).expect("no Script component");
    let code = s.inline.as_ref().expect("should have inline code");

    assert!(code.contains("i < 3"), "raw '<' should survive in script body: {}", code);
    assert!(code.contains("i && true"), "raw '&&' should survive in script body: {}", code);
}

#[test]
fn parse_malformed_xml_returns_error() {
    let mut world = make_world();
    let result = parse_xml(&mut world, "<unclosed");
    assert!(result.is_err());
}

#[test]
fn parse_empty_string_returns_error() {
    let mut world = make_world();
    let result = parse_xml(&mut world, "");
    assert!(result.is_err());
}

#[test]
fn parse_unknown_wrapper_tag_keeps_stack_in_sync() {
    let mut world = make_world();
    let xml = r#"<hsml><unknown><space><box/></space></unknown></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    let hier = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();

    let root_children = &hier.get(root).unwrap().children;
    assert_eq!(root_children.len(), 1);
    let space = root_children[0];
    assert_eq!(tags.get(space).unwrap().0, "space");
    assert_eq!(hier.get(space).unwrap().children.len(), 1);
    let child = hier.get(space).unwrap().children[0];
    assert_eq!(tags.get(child).unwrap().0, "box");
}

#[test]
fn parse_unknown_sibling_does_not_drop_following_known_nodes() {
    let mut world = make_world();
    let xml = r#"<hsml><space/><unknown></unknown><space/></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    let hier = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();
    let children = &hier.get(root).unwrap().children;
    assert_eq!(children.len(), 2);
    assert_eq!(tags.get(children[0]).unwrap().0, "space");
    assert_eq!(tags.get(children[1]).unwrap().0, "space");
}

// ─── serialize_xml ───────────────────────────────────────────────────────────

#[test]
fn serialize_root_tag() {
    let mut world = make_world();
    let xml = r#"<hsml></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");
    let serialized = serialize_xml(&world, root);
    assert!(serialized.contains("hsml"));
}

#[test]
fn serialize_preserves_child_tags() {
    let mut world = make_world();
    let xml = r#"<hsml><space><box/></space></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");
    let serialized = serialize_xml(&world, root);
    assert!(serialized.contains("space"));
    assert!(serialized.contains("box"));
}

#[test]
fn roundtrip_node_count() {
    let mut world = make_world();
    let xml = r#"<hsml><space><box x="1"/><text value="Hi"/></space></hsml>"#;
    let root = parse_xml(&mut world, xml).expect("parse failed");

    // Root hsml → space → 2 children = 4 entities total
    use specs::{Join, WorldExt};
    let hier = world.read_storage::<Hierarchy>();
    let count = (&world.entities(), &hier).join().count();
    assert_eq!(count, 4);
}
