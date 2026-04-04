use specs::prelude::*;
use specs_derive::Component;

#[derive(Component, Debug, Clone,Default)]
pub struct Model {
    pub src: Option<String>,
}
impl Model {
    fn default()-> Model{
        Model{src: None}
    }
}

#[derive(Component, Debug, Clone,Default)]
pub struct Script {
    pub src: Option<String>,
}
impl Script {
    fn default()-> Script{
        Script{src: None}
    }
}
#[derive(Component, Debug, Clone,Default)]
pub struct Include {
    pub src: Option<String>,
}
impl Include {
    fn default()-> Include{
        Include{src: None}
    }
}