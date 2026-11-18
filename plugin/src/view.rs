//! Small builder over the host's UI element tree.

use crate::astrobox::psys_host_v4::ui;

#[derive(Clone, Copy)]
pub enum On {
    Click,
    Change,
}

pub struct View {
    kind: ui::ElementType,
    content: Option<String>,
    props: Vec<(String, String)>,
    styles: Vec<Style>,
    events: Vec<(On, String)>,
    children: Vec<View>,
}

enum Style {
    Flex,
    Row,
    Column,
    Gap(u32),
    Padding(u32),
    MarginTop(u32),
    AlignCenter,
    AlignStart,
    Bg(String),
    Color(String),
    FontSize(u32),
    Width(u32),
    WidthFull,
    Height(u32),
    MaxWidth(u32),
    Radius(u32),
    Grow,
    Shrink,
    Disabled,
}

impl View {
    pub fn new(kind: ui::ElementType, content: Option<&str>) -> Self {
        Self {
            kind,
            content: content.map(str::to_string),
            props: Vec::new(),
            styles: Vec::new(),
            events: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::new(ui::ElementType::P, Some(&text.into()))
    }

    pub fn button(label: &str, event: &str) -> Self {
        Self::new(ui::ElementType::Button, Some(label))
            .prop("size", "2")
            .on(On::Click, event)
    }

    pub fn prop(mut self, key: &str, value: &str) -> Self {
        self.props.push((key.to_string(), value.to_string()));
        self
    }

    pub fn on(mut self, event: On, id: &str) -> Self {
        self.events.push((event, id.to_string()));
        self
    }

    pub fn child(mut self, child: View) -> Self {
        self.children.push(child);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = View>) -> Self {
        self.children.extend(children);
        self
    }

    pub fn maybe(self, child: Option<View>) -> Self {
        match child {
            Some(child) => self.child(child),
            None => self,
        }
    }

    fn style(mut self, style: Style) -> Self {
        self.styles.push(style);
        self
    }

    pub fn row(self) -> Self {
        self.style(Style::Flex).style(Style::Row)
    }

    pub fn column(self) -> Self {
        self.style(Style::Flex).style(Style::Column)
    }

    pub fn gap(self, gap: u32) -> Self {
        self.style(Style::Gap(gap))
    }

    pub fn padding(self, padding: u32) -> Self {
        self.style(Style::Padding(padding))
    }

    pub fn margin_top(self, margin: u32) -> Self {
        self.style(Style::MarginTop(margin))
    }

    pub fn align_center(self) -> Self {
        self.style(Style::AlignCenter)
    }

    pub fn align_start(self) -> Self {
        self.style(Style::AlignStart)
    }


    pub fn bg(self, color: &str) -> Self {
        self.style(Style::Bg(color.to_string()))
    }

    pub fn color(self, color: &str) -> Self {
        self.style(Style::Color(color.to_string()))
    }

    pub fn font_size(self, size: u32) -> Self {
        self.style(Style::FontSize(size))
    }

    pub fn width(self, width: u32) -> Self {
        self.style(Style::Width(width))
    }

    pub fn width_full(self) -> Self {
        self.style(Style::WidthFull)
    }

    pub fn height(self, height: u32) -> Self {
        self.style(Style::Height(height))
    }

    pub fn max_width(self, width: u32) -> Self {
        self.style(Style::MaxWidth(width))
    }

    pub fn radius(self, radius: u32) -> Self {
        self.style(Style::Radius(radius))
    }

    pub fn grow(self) -> Self {
        self.style(Style::Grow).style(Style::Shrink)
    }

    pub fn disabled(self) -> Self {
        self.style(Style::Disabled)
    }

    pub fn disabled_if(self, disabled: bool) -> Self {
        if disabled { self.disabled() } else { self }
    }

    pub fn into_element(self) -> ui::Element {
        let mut element = ui::Element::new(self.kind, self.content.as_deref());
        for (key, value) in &self.props {
            element = element.prop(key, value);
        }
        for style in self.styles {
            element = match style {
                Style::Flex => element.flex(),
                Style::Row => element.flex_direction(ui::FlexDirection::Row),
                Style::Column => element.flex_direction(ui::FlexDirection::Column),
                Style::Gap(v) => element.gap(v),
                Style::Padding(v) => element.padding(v),
                Style::MarginTop(v) => element.margin_top(v),
                Style::AlignCenter => element.align_center(),
                Style::AlignStart => element.align_start(),
                Style::Bg(c) => element.bg(&c),
                Style::Color(c) => element.text_color(&c),
                Style::FontSize(v) => element.size(v),
                Style::Width(v) => element.width(v),
                Style::WidthFull => element.width_full(),
                Style::Height(v) => element.height(v),
                Style::MaxWidth(v) => element.max_width(v),
                Style::Radius(v) => element.radius(v),
                Style::Grow => element.flex_grow(1.0),
                Style::Shrink => element.flex_shrink(1.0),
                Style::Disabled => element.disabled(),
            };
        }
        for (event, id) in &self.events {
            let event = match event {
                On::Click => ui::Event::Click,
                On::Change => ui::Event::Change,
            };
            element = element.on(event, id);
        }
        for child in self.children {
            element = element.child(child.into_element());
        }
        element
    }
}
