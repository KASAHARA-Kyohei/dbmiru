use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, IntoElement, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    Render, ShapedLine, SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window,
    actions, div, fill, hsla, prelude::*, px, rgb, rgba,
};
use unicode_segmentation::UnicodeSegmentation;

const OBSCURED_CHAR: &str = "•";
const KEY_CONTEXT: &str = "TextInput";

actions!(
    text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        ShowCharacterPalette,
        Paste,
        Cut,
        Copy,
    ]
);

pub struct TextInput {
    focus_handle: FocusHandle,
    content: String,
    placeholder: String,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    obscure: bool,
}

impl TextInput {
    pub fn new(cx: &mut Context<Self>, initial: &str, placeholder: &str) -> Self {
        Self::register_keybindings(cx);
        Self {
            focus_handle: cx.focus_handle(),
            content: initial.to_owned(),
            placeholder: placeholder.to_owned(),
            selected_range: initial.len()..initial.len(),
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            obscure: false,
        }
    }

    pub fn with_obscured(mut self, obscure: bool) -> Self {
        self.obscure = obscure;
        self
    }

    pub fn set_text(&mut self, value: &str) {
        self.content = value.to_owned();
        let end = self.content.len();
        self.selected_range = end..end;
    }

    pub fn text(&self) -> String {
        self.content.clone()
    }

    pub fn clear(&mut self) {
        self.content.clear();
        self.selected_range = 0..0;
    }

    fn schedule_redraw(window: &mut Window, cx: &mut Context<Self>) {
        let entity_id = cx.entity_id();
        window.on_next_frame(move |_, app| {
            app.notify(entity_id);
        });
    }

    fn obscured_offset(&self, offset: usize) -> usize {
        let char_count = self.content[..offset].chars().count();
        char_count * OBSCURED_CHAR.len()
    }

    fn obscured_range(&self, range: &Range<usize>) -> Range<usize> {
        self.obscured_offset(range.start)..self.obscured_offset(range.end)
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            self.content[0..range.start].to_owned() + new_text + &self.content[range.end..];
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        Self::schedule_redraw(window, cx);
        cx.notify();
    }
    fn register_keybindings(cx: &mut Context<Self>) {
        cx.bind_keys([
            KeyBinding::new("backspace", Backspace, Some(KEY_CONTEXT)),
            KeyBinding::new("delete", Delete, Some(KEY_CONTEXT)),
            KeyBinding::new("left", Left, Some(KEY_CONTEXT)),
            KeyBinding::new("right", Right, Some(KEY_CONTEXT)),
            KeyBinding::new("shift-left", SelectLeft, Some(KEY_CONTEXT)),
            KeyBinding::new("shift-right", SelectRight, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-a", SelectAll, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-a", SelectAll, Some(KEY_CONTEXT)),
            KeyBinding::new("home", Home, Some(KEY_CONTEXT)),
            KeyBinding::new("end", End, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-left", Home, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-right", End, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-c", Copy, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-c", Copy, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-v", Paste, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-v", Paste, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-x", Cut, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-x", Cut, Some(KEY_CONTEXT)),
        ]);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_selecting = true;

        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        cx.notify();
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        line.closest_index_for_x(position.x - bounds.left())
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        let start = self.offset_from_utf16(range_utf16.start);
        let end = self.offset_from_utf16(range_utf16.end);
        start..end
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        let start = self.utf16_offset(range.start);
        let end = self.utf16_offset(range.end);
        start..end
    }

    fn utf16_offset(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        for (idx, ch) in self.content.char_indices() {
            if idx >= offset {
                break;
            }
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn offset_from_utf16(&self, utf16_offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.content.chars() {
            if utf16_count >= utf16_offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_text_in_range(range_utf16, new_text, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            self.content[0..range.start].to_owned() + new_text + &self.content[range.end..];
        if let Some(selected_range_utf16) = new_selected_range_utf16 {
            self.selected_range = self.range_from_utf16(&selected_range_utf16);
        } else {
            self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        }
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        Self::schedule_redraw(window, cx);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let mut min_x = None;
        let mut max_x = None;
        if let Some(line) = self.last_layout.as_ref() {
            min_x = Some(line.x_for_index(range.start));
            max_x = Some(line.x_for_index(range.end));
        }
        match (min_x, max_x) {
            (Some(min_x), Some(max_x)) => Some(Bounds::from_corners(
                Point::new(bounds.left() + min_x, bounds.top()),
                Point::new(bounds.left() + max_x, bounds.bottom()),
            )),
            _ => {
                cx.notify();
                None
            }
        }
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        if let Some(bounds) = self.last_bounds
            && let Some(line) = self.last_layout.as_ref()
            && bounds.contains(&point)
        {
            return Some(line.closest_index_for_x(point.x - bounds.left()));
        }
        None
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .key_context("TextInput")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .bg(rgb(0x111827))
            .text_color(rgb(0xf8fafc))
            .child(
                div()
                    .h(px(36.))
                    .w_full()
                    .px_3()
                    .rounded_lg()
                    .items_center()
                    .bg(rgb(0x0b1120))
                    .border_1()
                    .border_color(rgb(0x1f2937))
                    .child(TextElement { input: cx.entity() }),
            )
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl TextElement {
    fn text_bounds(bounds: Bounds<Pixels>, line_height: Pixels) -> (Bounds<Pixels>, Pixels) {
        let available_height = bounds.bottom() - bounds.top();
        let text_height = if available_height < line_height {
            available_height
        } else {
            line_height
        };
        let offset = (available_height - text_height) * 0.5;
        let text_top = bounds.top() + offset;

        let left = bounds.left();
        let right = bounds.right();
        (
            Bounds::from_corners(
                Point::new(left, text_top),
                Point::new(right, text_top + text_height),
            ),
            text_height,
        )
    }
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let style = window.text_style();
        let mut display_text = input.content.clone();
        let mut selected_range = input.selected_range.clone();
        let mut cursor = input.cursor_offset();
        let mut marked_range = input.marked_range.clone();
        let line_height = window.line_height();
        let (text_bounds, text_height) = Self::text_bounds(bounds, line_height);

        if input.obscure && !display_text.is_empty() {
            let char_count = input.content.chars().count();
            display_text = OBSCURED_CHAR.repeat(char_count);
            selected_range = input.obscured_range(&input.selected_range);
            cursor = if input.selection_reversed {
                input.obscured_offset(input.selected_range.start)
            } else {
                input.obscured_offset(input.selected_range.end)
            };
            marked_range = input
                .marked_range
                .as_ref()
                .map(|range| input.obscured_range(range));
        }

        let (display_text, text_color) = if display_text.is_empty() {
            (input.placeholder.clone(), hsla(0., 0., 1., 0.35))
        } else {
            (display_text, style.color)
        };

        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked_range) = marked_range.as_ref() {
            vec![
                TextRun {
                    len: marked_range.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked_range.end - marked_range.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len() - marked_range.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let shared_text: SharedString = display_text.clone().into();
        let line = window
            .text_system()
            .shape_line(shared_text, font_size, &runs, None);

        let cursor_pos = line.x_for_index(cursor);
        let (selection, cursor) = if selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        Point::new(text_bounds.left() + cursor_pos, text_bounds.top()),
                        gpui::size(px(2.), text_height),
                    ),
                    gpui::blue(),
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        Point::new(
                            text_bounds.left() + line.x_for_index(selected_range.start),
                            text_bounds.top(),
                        ),
                        Point::new(
                            text_bounds.left() + line.x_for_index(selected_range.end),
                            text_bounds.bottom(),
                        ),
                    ),
                    rgba(0x3311ff30),
                )),
                None,
            )
        };

        PrepaintState {
            line: Some(line),
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection)
        }
        let line = prepaint.line.take().unwrap();
        let line_height = window.line_height();
        let (text_bounds, text_height) = Self::text_bounds(bounds, line_height);
        line.paint(text_bounds.origin, text_height, window, cx)
            .unwrap();

        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        self.input.update(cx, |input, _cx| {
            input.last_layout = Some(line);
            input.last_bounds = Some(text_bounds);
        });
    }
}
