use lazy_static::lazy_static;

use std::collections::HashMap;

use super::element::ElementType;

lazy_static! {
    pub static ref TAGS_VALUES: HashMap<&'static str, Vec<ElementType>> = {
        let mut m = HashMap::new();
        m.insert("hsml", vec![ElementType::Node, ElementType::Element, ElementType::HSMLElement]);
        m.insert("head", vec![ElementType::Node]);
        m.insert("name", vec![ElementType::Node]);
        m.insert("state", vec![ElementType::Node]);
        m.insert("meta", vec![ElementType::Node]);
        m.insert("script", vec![ElementType::Node, ElementType::HSMLElement]);
        m.insert("include", vec![ElementType::Node, ElementType::Element, ElementType::HSMLElement]);
        m.insert("space", vec![ElementType::Node, ElementType::Element, ElementType::HSMLElement]);
        m.insert("posezone", vec![ElementType::Node, ElementType::Element]);
        m.insert("model", vec![ElementType::Node, ElementType::Element, ElementType::HSMLElement]);
        // `group` es el tag nativo HSML para agrupación. `div` se mantiene como
        // alias por compatibilidad con costumbre de navegador.
        m.insert("group", vec![ElementType::Node, ElementType::Element, ElementType::HSMLElement]);
        m.insert("div", vec![ElementType::Node, ElementType::Element, ElementType::HSMLElement]);

        // Visual elements
        m.insert("text", vec![ElementType::Node, ElementType::Element]);
        m.insert("box", vec![ElementType::Node, ElementType::Element]);
        m.insert("sphere", vec![ElementType::Node, ElementType::Element]);
        m.insert("cylinder", vec![ElementType::Node, ElementType::Element]);
        m.insert("plane", vec![ElementType::Node, ElementType::Element]);
        m.insert("image", vec![ElementType::Node, ElementType::Element]);
        m.insert("skybox", vec![ElementType::Node, ElementType::Element]);

        m
    };
}
