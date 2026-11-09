//! A row whose children share the width equally and all take the height of
//! the tallest one. iced's own row can only stretch children to fill a known
//! height, which a scrolling page doesn't have, so this measures first.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Shell, Widget, overlay, renderer};
use iced::event::{self, Event};
use iced::{Element, Length, Point, Rectangle, Size, Vector, mouse};

pub struct EqualRow<'a, Message, Theme, Renderer> {
    children: Vec<Element<'a, Message, Theme, Renderer>>,
    /// Cells per row; missing ones are left empty so widths stay the same.
    cells: usize,
    spacing: f32,
}

impl<'a, Message, Theme, Renderer> EqualRow<'a, Message, Theme, Renderer> {
    /// Children should be shrink-height; each is given the tallest one's height.
    pub fn new(children: Vec<Element<'a, Message, Theme, Renderer>>, cells: usize, spacing: f32) -> Self {
        let cells = cells.max(children.len()).max(1);
        EqualRow { children, cells, spacing }
    }
}

impl<Message, Theme, Renderer: renderer::Renderer> Widget<Message, Theme, Renderer> for EqualRow<'_, Message, Theme, Renderer> {
    fn children(&self) -> Vec<Tree> {
        self.children.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.children);
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(&self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let width = limits.max().width;
        let n = self.cells as f32;
        let cell = ((width - self.spacing * (n - 1.0)) / n).max(0.0);

        let measure = layout::Limits::new(Size::new(cell, 0.0), Size::new(cell, f32::INFINITY));
        let tallest = self
            .children
            .iter()
            .zip(&mut tree.children)
            .map(|(child, state)| child.as_widget().layout(state, renderer, &measure).size().height)
            .fold(0.0, f32::max);

        let fixed = layout::Limits::new(Size::new(cell, tallest), Size::new(cell, tallest));
        let nodes = self
            .children
            .iter()
            .zip(&mut tree.children)
            .enumerate()
            .map(|(i, (child, state))| child.as_widget().layout(state, renderer, &fixed).move_to(Point::new(i as f32 * (cell + self.spacing), 0.0)))
            .collect();
        layout::Node::with_children(Size::new(width, tallest), nodes)
    }

    fn operate(&self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        operation.container(None, layout.bounds(), &mut |operation| {
            for ((child, state), layout) in self.children.iter().zip(&mut tree.children).zip(layout.children()) {
                child.as_widget().operate(state, layout, renderer, operation);
            }
        });
    }

    fn on_event(
        &mut self,
        tree: &mut Tree,
        event: Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) -> event::Status {
        self.children
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
            .map(|((child, state), layout)| {
                child.as_widget_mut().on_event(state, event.clone(), layout, cursor, renderer, clipboard, shell, viewport)
            })
            .fold(event::Status::Ignored, event::Status::merge)
    }

    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        self.children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((child, state), layout)| child.as_widget().mouse_interaction(state, layout, cursor, viewport, renderer))
            .max()
            .unwrap_or_default()
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        for ((child, state), layout) in self.children.iter().zip(&tree.children).zip(layout.children()) {
            child.as_widget().draw(state, renderer, theme, style, layout, cursor, viewport);
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        overlay::from_children(&mut self.children, tree, layout, renderer, translation)
    }
}

impl<'a, Message: 'a, Theme: 'a, Renderer: renderer::Renderer + 'a> From<EqualRow<'a, Message, Theme, Renderer>> for Element<'a, Message, Theme, Renderer> {
    fn from(row: EqualRow<'a, Message, Theme, Renderer>) -> Self {
        Element::new(row)
    }
}
