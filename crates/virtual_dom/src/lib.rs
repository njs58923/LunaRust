use std::collections::HashMap;

use anyhow::Result;
use dom::{
    element::{Attrs, ElementType, Hierarchy, Tag, Text, Transform2, Vec3},
    hsml::{Include, Model, Script},
    tags::TAGS_VALUES,
    TRANSFORM_POSITION, TRANSFORM_ROTATION, TRANSFORM_SCALE,
};
use quick_xml::events::attributes::Attributes as XmlAttributes;
use quick_xml::events::Event;
use quick_xml::Reader;
use specs::{Builder, Entity, ReadStorage, World, WorldExt};

pub mod dom;

use dom::tags;

fn find_tag_end(xml: &str, start: usize) -> Option<usize> {
    let bytes = xml.as_bytes();
    let mut i = start;
    let mut quote: Option<u8> = None;

    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) => {
                if b == q {
                    quote = None;
                }
            }
            None => {
                if b == b'\'' || b == b'"' {
                    quote = Some(b);
                } else if b == b'>' {
                    return Some(i);
                }
            }
        }
        i += 1;
    }

    None
}

fn decode_basic_xml_entities(input: &str) -> String {
    input
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn cdata_safe(input: &str) -> String {
    input.replace("]]>", "]]]]><![CDATA[>")
}

/// Emula el comportamiento "raw text" de <script> en navegadores,
/// aunque el documento completo siga siendo parseado como XML.
///
/// - Detecta <script>...</script>
/// - Si no es self-closing, envuelve el cuerpo en CDATA
/// - Además decodifica entidades XML comunes para mantener compatibilidad
///   con scripts viejos escritos como XML puro (&lt;, &amp;&amp;, etc.)
fn normalize_inline_script_blocks(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len() + 64);
    let mut cursor = 0usize;

    while let Some(rel_start) = xml[cursor..].find("<script") {
        let start = cursor + rel_start;
        out.push_str(&xml[cursor..start]);

        let Some(open_end) = find_tag_end(xml, start) else {
            out.push_str(&xml[start..]);
            return out;
        };

        let open_tag = &xml[start..=open_end];
        out.push_str(open_tag);

        let is_self_closing = open_tag.trim_end().ends_with("/>");
        if is_self_closing {
            cursor = open_end + 1;
            continue;
        }

        let body_start = open_end + 1;
        let Some(rel_close) = xml[body_start..].find("</script>") else {
            out.push_str(&xml[body_start..]);
            return out;
        };
        let close_start = body_start + rel_close;
        let body = &xml[body_start..close_start];
        let trimmed = body.trim();

        if trimmed.starts_with("<![CDATA[") {
            out.push_str(body);
        } else {
            let normalized = decode_basic_xml_entities(body);
            out.push_str("<![CDATA[");
            out.push_str(&cdata_safe(&normalized));
            out.push_str("]]>");
        }

        out.push_str("</script>");
        cursor = close_start + "</script>".len();
    }

    out.push_str(&xml[cursor..]);
    out
}

/// Carga el XML desde una URL y retorna el contenido como String.
pub async fn load_xml_from_url(url: &str) -> Result<String> {
    let response = reqwest::get(url).await?.error_for_status()?;
    let text = response.text().await?;
    Ok(text)
}

fn read_attributes(attributes: XmlAttributes) -> HashMap<String, String> {
    let mut node: HashMap<String, String> = HashMap::new();
    for attr in attributes {
        if let Ok(attr) = attr {
            let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
            let value = attr
                .unescape_value()
                .unwrap_or_else(|_| "".into())
                .to_string();
            node.insert(key, value);
        }
    }

    return node;
}
fn read_vec3(attributes: &HashMap<String, String>, x: &str, y: &str, z: &str, d: &str) -> Vec3 {
    return Vec3 {
        x: attributes
            .get(x)
            .unwrap_or(&d.to_owned())
            .parse::<f32>()
            .unwrap_or(0.0),
        y: attributes
            .get(y)
            .unwrap_or(&d.to_owned())
            .parse::<f32>()
            .unwrap_or(0.0),
        z: attributes
            .get(z)
            .unwrap_or(&d.to_owned())
            .parse::<f32>()
            .unwrap_or(0.0),
    };
}

fn read_str(attributes: &HashMap<String, String>, key: &String, default: &String) -> String {
    return attributes.get(key).unwrap_or(default).clone();
}

fn read_some_str(attributes: &HashMap<String, String>, key: &str, default: &str) -> Option<String> {
    if let Some(value) = attributes.get(key) {
        return Some(value.clone());
    }
    if default == "" {
        return None;
    }
    return Some(default.to_owned().clone());
}

/// Parsea el XML y construye el DOM virtual utilizando HSMLElement.
/// Este parser es simple y asume un XML bien formado.
pub fn parse_xml(world: &mut World, xml_content: &str) -> Result<Entity> {
    struct OpenNode {
        tag: String,
        entity: Option<Entity>,
    }

    let normalized_xml = normalize_inline_script_blocks(xml_content);
    let mut reader = Reader::from_str(&normalized_xml);
    reader.trim_text(true);
    let mut node_stack: Vec<Entity> = Vec::new();
    let mut open_stack: Vec<OpenNode> = Vec::new();
    let mut root: Option<Entity> = None;
    let mut buf: Vec<u8> = Vec::new();
    let mut text_accum: String = String::new();

    fn apply(
        world: &mut World,
        tag: &String,
        attributes: &HashMap<String, String>,
    ) -> Option<Entity> {
        let mut node_build = world.create_entity();

        // println!("T: {:?}", &tag);
        if let Some(node_list) = TAGS_VALUES.get(&tag as &str) {
            for element in node_list.iter() {
                match element {
                    ElementType::Node => {
                        node_build = node_build.with(Tag(tag.to_string()));
                        node_build = node_build.with(Hierarchy::default());
                        if tag == "[TEXT]" {
                            node_build = node_build.with(Text(tag.to_string()));
                        }
                        node_build = node_build.with(Attrs(attributes.clone()));
                    }
                    ElementType::Element => {
                        let scale_value: Vec3;

                        if let Some(value) = attributes.get("s") {
                            scale_value = Vec3 {
                                x: value.parse::<f32>().unwrap_or(1.0),
                                y: value.parse::<f32>().unwrap_or(1.0),
                                z: value.parse::<f32>().unwrap_or(1.0),
                            }
                        } else {
                            scale_value = read_vec3(
                                &attributes,
                                TRANSFORM_SCALE[0],
                                TRANSFORM_SCALE[1],
                                TRANSFORM_SCALE[2],
                                "1",
                            )
                        }

                        node_build = node_build.with(Transform2 {
                            position: read_vec3(
                                &attributes,
                                TRANSFORM_POSITION[0],
                                TRANSFORM_POSITION[1],
                                TRANSFORM_POSITION[2],
                                "0",
                            ),
                            rotation: read_vec3(
                                &attributes,
                                TRANSFORM_ROTATION[0],
                                TRANSFORM_ROTATION[1],
                                TRANSFORM_ROTATION[2],
                                "0",
                            ),
                            scale: scale_value,
                        });
                    }
                    ElementType::HSMLElement => {
                        if tag == "model" {
                            node_build = node_build.with(Model {
                                src: read_some_str(&attributes, "src", ""),
                            });
                        } else if tag == "script" {
                            node_build = node_build.with(Script {
                                src: read_some_str(&attributes, "src", ""),
                                inline: None,
                            });
                        } else if tag == "include" {
                            node_build = node_build.with(Include {
                                src: read_some_str(&attributes, "src", ""),
                            });
                        }
                    }
                }
            }
            return Some(node_build.build());
        }
        return None;
    }

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let attributes = read_attributes(e.attributes());

                let entity = apply(world, &tag, &attributes);
                if let Some(entity) = entity {
                    node_stack.push(entity);
                }
                open_stack.push(OpenNode { tag, entity });
                text_accum.clear();
            }
            Ok(Event::Empty(ref e)) => {
                // Manejo de etiquetas autocontenidas.
                let tag = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let attributes = read_attributes(e.attributes());

                if let Some(entity) = apply(world, &tag, &attributes) {
                    let parent = node_stack.last_mut();
                    if let Some(parent) = parent {
                        Hierarchy::add_child(world, parent.clone(), entity);
                    } else {
                        root = Some(entity);
                    }
                }
            }
            Ok(Event::End(_)) => {
                let Some(open) = open_stack.pop() else {
                    text_accum.clear();
                    continue;
                };

                if let Some(node) = open.entity {
                    if open.tag == "script" && !text_accum.trim().is_empty() {
                        let mut scripts = world.write_storage::<Script>();
                        if let Some(script) = scripts.get_mut(node) {
                            script.inline = Some(text_accum.trim().to_string());
                        }
                    }

                    match node_stack.pop() {
                        Some(popped) if popped == node => {}
                        Some(popped) => {
                            return Err(anyhow::anyhow!(
                                "Error leyendo XML: pila de nodos desincronizada al cerrar <{}> (tope={:?}, esperado={:?})",
                                open.tag,
                                popped,
                                node
                            ));
                        }
                        None => {
                            return Err(anyhow::anyhow!(
                                "Error leyendo XML: pila de nodos vacía al cerrar <{}>",
                                open.tag
                            ));
                        }
                    }

                    if let Some(parent) = node_stack.last().copied() {
                        Hierarchy::add_child(world, parent, node);
                    } else {
                        root = Some(node);
                    }
                }

                text_accum.clear();
            }
            Ok(Event::Text(ref e)) => {
                if open_stack.last().map(|open| open.tag.as_str()) == Some("script") {
                    let txt = e.unescape().unwrap_or_default();
                    text_accum.push_str(&txt);
                }
            }
            Ok(Event::CData(ref e)) => {
                if open_stack.last().map(|open| open.tag.as_str()) == Some("script") {
                    let txt = String::from_utf8_lossy(e.as_ref());
                    text_accum.push_str(&txt);
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("Error leyendo XML: {:?}", e)),
            _ => {}
        }
        buf.clear();
    }
    root.ok_or_else(|| anyhow::anyhow!("No se encontró un nodo raíz en el XML"))
}

/// Serializa el DOM virtual a XML utilizando HSMLElement.
pub fn serialize_xml(world: &World, entity: Entity) -> String {
    // Obtener los storages UNA SOLA VEZ al inicio:
    let tags = world.read_storage::<Tag>();
    let attrs = world.read_storage::<Attrs>();
    let transforms = world.read_storage::<Transform2>();
    let hierarchies = world.read_storage::<Hierarchy>();

    // Llamar función interna recursiva:
    serialize_xml_internal(entity, &tags, &attrs, &transforms, &hierarchies)
}

// Esta función interna evita conflictos de préstamos porque ya no usa `world`.
fn serialize_xml_internal(
    entity: Entity,
    tags: &ReadStorage<Tag>,
    attrs: &ReadStorage<Attrs>,
    transforms: &ReadStorage<Transform2>,
    hierarchies: &ReadStorage<Hierarchy>,
) -> String {
    let mut xml = String::new();

    if let Some(tag) = tags.get(entity) {
        xml.push_str(&format!("<{}", tag.0));

        if let Some(args) = attrs.get(entity) {
            for (key, value) in &args.0 {
                xml.push_str(&format!(" {}=\"{}\"", key, value));
            }
        }

        fn str_vec3(p: Vec3, x: &str, y: &str, z: &str, d: f32) -> Option<String> {
            let mut attrs = String::new();

            if p.x != d {
                attrs.push_str(&format!(" {}=\"{}\"", x, p.x));
            }
            if p.y != d {
                attrs.push_str(&format!(" {}=\"{}\"", y, p.y));
            }
            if p.z != d {
                attrs.push_str(&format!(" {}=\"{}\"", z, p.z));
            }

            if attrs.len() > 0 {
                return Some(attrs);
            }
            return None;
        }

        if let Some(transform) = transforms.get(entity) {
            if let Some(atrs) = str_vec3(
                transform.position,
                TRANSFORM_POSITION[0],
                TRANSFORM_POSITION[1],
                TRANSFORM_POSITION[2],
                0.0,
            ) {
                xml.push_str(&atrs)
            }
            if let Some(atrs) = str_vec3(
                transform.rotation,
                TRANSFORM_ROTATION[0],
                TRANSFORM_ROTATION[1],
                TRANSFORM_ROTATION[2],
                0.0,
            ) {
                xml.push_str(&atrs)
            }
            if let Some(atrs) = str_vec3(
                transform.scale,
                TRANSFORM_SCALE[0],
                TRANSFORM_SCALE[1],
                TRANSFORM_SCALE[2],
                1.0,
            ) {
                xml.push_str(&atrs)
            }

            // Puedes continuar con rotation y scale...
        }

        if let Some(hierarchy) = hierarchies.get(entity) {
            if hierarchy.children.is_empty() {
                xml.push_str("/>");
            } else {
                xml.push_str(">");
                for child in &hierarchy.children {
                    xml.push_str(&serialize_xml_internal(
                        *child,
                        tags,
                        attrs,
                        transforms,
                        hierarchies,
                    ));
                }
                xml.push_str(&format!("</{}>", tags.get(entity).unwrap().0));
            }
        } else {
            xml.push_str("/>");
        }
    }

    xml
}
