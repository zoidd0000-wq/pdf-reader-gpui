use gpui::prelude::FluentBuilder;
use gpui::{
    AlignItems, AppContext, Context, Empty, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Pixels, Point, Render, ScrollHandle, ScrollWheelEvent,
    SharedString, StatefulInteractiveElement, StyleRefinement, Styled, Window, div, point, px,
};
use gpui_component::button::Button;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::tooltip::Tooltip;
use gpui_component::{ActiveTheme, Icon, IconName, StyledExt};
use std::cmp::Ordering;
use std::ops::Sub;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
#[cfg(not(target_family = "wasm"))]
use std::time::Instant;
#[cfg(target_family = "wasm")]
use web_time::Instant;

#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = tabs)]
pub struct CloseTab;

#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = tabs)]
pub struct CreateTab;

#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = tabs)]
pub struct NextTab;

#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = tabs)]
pub struct PrevTab;

pub struct SmoothScrollState {
    /// Animation state
    animating: bool,
    /// The scroll offset where the animation started.
    start_offset: Point<Pixels>,
    /// The last scroll offset acknowledged by this smooth scroll state.
    last_set_offset: Point<Pixels>,
    /// The scroll offset that the animation will finish at.
    target_offset: Point<Pixels>,
    /// Animation started at this time. Note that if a new target offset is provided during the
    /// animation then this time might be recalculated to provide a smooth animation.
    start_time: Instant,
    /// Animation duration
    duration: Duration,
    /// True if one of the `scroll_to_` method on the scroll handle was called, for example
    /// [`ScrollHandle::scroll_to_item`]. These don't set the offset until 2 frames after
    /// requested so we need to request a new update then to get and override that new offset.
    requested_async_scroll: u32,
    requested_scroll_to_item: Option<usize>,
}
impl SmoothScrollState {
    pub fn new() -> Self {
        Self {
            animating: false,
            start_offset: point(px(0.), px(0.)),
            last_set_offset: point(px(0.), px(0.)),
            target_offset: point(px(0.), px(0.)),
            start_time: Instant::now(),
            duration: Duration::from_millis(1500),
            requested_async_scroll: 0,
            requested_scroll_to_item: None,
        }
    }

    // Easing function (ease-in-out)
    fn ease_in_out(t: f32) -> f32 {
        if t < 0.5 {
            2.0 * t * t
        } else {
            -1.0 + (4.0 - 2.0 * t) * t
        }
    }

    fn bound_scroll(scroll_handle: &ScrollHandle, offset: Point<Pixels>) -> Point<Pixels> {
        let bounds = scroll_handle.max_offset();
        let safe_x_range = (-bounds.width).min(px(0.0))..px(0.);
        let safe_y_range = (-bounds.height).min(px(0.0))..px(0.);
        point(
            offset.x.clamp(safe_x_range.start, safe_x_range.end),
            offset.y.clamp(safe_y_range.start, safe_y_range.end),
        )
    }

    /// Scrolls the minimal amount to either ensure that the child is
    /// fully visible or the top element of the view depends on the
    /// scroll strategy
    pub fn scroll_to_item<T: 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<T>,
        scroll_handle: &ScrollHandle,
        index: usize,
    ) {
        if let Some(bounds) = scroll_handle
            .bounds_for_item(index)
            .filter(|item| !item.is_empty())
        {
            let mut scroll_offset = scroll_handle.offset();
            let all_bounds = scroll_handle.bounds();

            if bounds.top() + scroll_offset.y < all_bounds.top() {
                scroll_offset.y = all_bounds.top() - bounds.top();
            } else if bounds.bottom() + scroll_offset.y > all_bounds.bottom() {
                scroll_offset.y = all_bounds.bottom() - bounds.bottom();
            }

            if bounds.left() + scroll_offset.x < all_bounds.left() {
                scroll_offset.x = all_bounds.left() - bounds.left();
            } else if bounds.right() + scroll_offset.x > all_bounds.right() {
                scroll_offset.x = all_bounds.right() - bounds.right();
            }

            scroll_handle.set_offset(scroll_offset);
            self.noticed_scroll(window, cx, scroll_handle);
        } else {
            // Item is new and hasn't been created yet:
            self.requested_scroll_to_item = Some(index);
            // Prevent infinite re-checking (we only try while requested_async_scroll is above 0.
            if self.requested_async_scroll == 0 {
                self.requested_async_scroll = 2;
            }
        }
    }

    /// Indicate that the newly set scroll offset is where we actually want to scroll.
    pub fn requested_async_scroll(&mut self) {
        self.requested_async_scroll = 2;
    }
    /// Start animation if target offset has changed.
    pub fn noticed_scroll<T: 'static>(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<T>,
        scroll_handle: &ScrollHandle,
    ) {
        let current_offset = Self::bound_scroll(scroll_handle, scroll_handle.offset());
        let diff = self.last_set_offset - current_offset;
        if diff.x.abs() > px(2.) || diff.y.abs() > px(2.) {
            self.start_offset = self.wanted_offset();
            self.start_scroll_to(current_offset);
            cx.notify();
        }
    }
    /// Assume that the offset was changed relative to the last set offset.
    pub fn noticed_scroll_wheel_event<T: 'static>(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<T>,
        scroll_handle: &ScrollHandle,
    ) {
        let current_offset = Self::bound_scroll(scroll_handle, scroll_handle.offset());

        if self.last_set_offset != current_offset {
            self.start_offset = self.wanted_offset();
            self.start_scroll_to(Self::bound_scroll(
                scroll_handle,
                self.target_offset + (current_offset - self.last_set_offset),
            ));
        }
        self.last_set_offset = current_offset;
    }
    /// Start animation
    pub fn start_scroll_to(&mut self, target_offset: Point<Pixels>) {
        if target_offset == self.target_offset {
            return;
        }
        // self.start_offset = start_offset;
        self.target_offset = target_offset;
        if self.animating {
            // Select a start time that gives the same progress percentage in order to not change
            // the animation "speed".
            let elapsed = Instant::now().duration_since(self.start_time);
            let mut progress = (elapsed.as_secs_f32() / self.duration.as_secs_f32()).min(1.0);
            // If more than half has passed then consider an earlier animation point with same speed
            // (i.e. 90% of progress has same speed as 10% of progress)
            if progress > 0.5 {
                progress = 1. - progress;
            }
            self.duration = Duration::from_millis(300);
            // Ensure at least half the time remains:
            self.start_time = Instant::now().sub(Duration::from_secs_f32(
                self.duration.as_secs_f32() * progress,
            ));
        } else {
            self.duration = Duration::from_millis(300);
            self.start_time = Instant::now();
        }
        self.animating = true;
    }

    pub fn is_complete(&self) -> bool {
        Instant::now().duration_since(self.start_time) >= self.duration
    }

    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// Gets the desired offset for the current time. If animating then this will calculate an
    /// interpolated offset
    pub fn wanted_offset(&self) -> Point<Pixels> {
        if !self.animating {
            return self.target_offset;
        }
        let elapsed = Instant::now().duration_since(self.start_time);
        if elapsed >= self.duration {
            self.target_offset
        } else {
            let progress = (elapsed.as_secs_f32() / self.duration.as_secs_f32()).min(1.0);
            let eased = Self::ease_in_out(progress);

            point(
                self.start_offset.x + (self.target_offset.x - self.start_offset.x) * eased,
                self.start_offset.y + (self.target_offset.y - self.start_offset.y) * eased,
            )
        }
    }

    pub fn preform_scroll<T: 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<T>,
        scroll_handle: &ScrollHandle,
    ) {
        if self.requested_async_scroll > 0 {
            if let Some(index) = self.requested_scroll_to_item.take() {
                self.scroll_to_item(window, cx, scroll_handle, index);
            }
            self.requested_async_scroll -= 1;
            self.noticed_scroll(window, cx, scroll_handle);
            if self.requested_async_scroll > 0 {
                window.request_animation_frame();
            }
        } else {
            // If we failed to scroll to the item for several frames, then forget about it:
            self.requested_scroll_to_item = None;
        }
        // Update animation if active
        if self.animating {
            let next_offset = self.wanted_offset();
            scroll_handle.set_offset(next_offset);
            self.last_set_offset = next_offset;
            if self.is_complete() {
                // Animation complete
                self.animating = false;
            } else {
                // Request next frame (pattern from scrollbar fade animation)
                window.request_animation_frame();
            }

            cx.notify();
        }
    }
}
impl Default for SmoothScrollState {
    fn default() -> Self {
        Self::new()
    }
}

pub trait TabData: 'static {
    fn label(&self) -> SharedString;
    fn full_path(&self) -> Arc<PathBuf>;
}

pub struct TabsView<T: 'static> {
    active_tab: usize,
    tabs: Vec<Option<T>>,
    scroll_handle: ScrollHandle,
    smooth_scroll: SmoothScrollState,
    on_tab_changed: Box<dyn Fn(&mut Window, &mut Context<Self>) + 'static>,
}
impl<T> TabsView<T> {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            active_tab: 0,
            tabs: vec![None],
            scroll_handle: ScrollHandle::new(),
            smooth_scroll: SmoothScrollState::new(),
            on_tab_changed: Box::new(|_window, _cx| {}),
        }
    }
    pub fn on_tab_changed(&mut self, handler: impl Fn(&mut Window, &mut Context<Self>) + 'static) {
        self.on_tab_changed = Box::new(handler);
    }

    pub fn create_tab(&mut self, data: Option<T>, window: &mut Window, cx: &mut Context<Self>) {
        self.active_tab = self.tabs.len();
        self.tabs.push(data);
        (self.on_tab_changed)(window, cx);
    }
    pub fn remove_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.tabs.len() {
            0 => return,
            1 => {
                self.tabs[0] = None;
                (self.on_tab_changed)(window, cx);
            }
            _ => {
                self.tabs.remove(index);
                if self.active_tab == index {
                    self.set_active_tab(index, window, cx);
                    self.scroll_to_active_tab(window, cx); // ensure the new tab is visible
                } else if index < self.active_tab {
                    self.active_tab -= 1;
                }
            }
        }
        cx.notify();
    }
    pub fn set_active_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active_tab = index.min(self.tabs.len().saturating_sub(1));
        (self.on_tab_changed)(window, cx);
    }

    pub fn scroll_to_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_tab == 0 {
            self.scroll_handle.set_offset(Point::default());
            self.smooth_scroll
                .noticed_scroll(window, cx, &self.scroll_handle);
        } else {
            self.smooth_scroll
                .scroll_to_item(window, cx, &self.scroll_handle, self.active_tab);

            // Previously used the inbuilt method:
            // self.scroll_handle.scroll_to_item(self.active_tab); // <- updates the scroll offset later
            // We need to get the scroll offset when it becomes available next frame:
            // self.smooth_scroll.requested_async_scroll();
        }
    }
    pub fn active_tab(&self) -> usize {
        self.active_tab
    }
    pub fn tabs_data(&self) -> &[Option<T>] {
        self.tabs.as_slice()
    }
    pub fn tabs_data_mut(&mut self) -> &mut [Option<T>] {
        self.tabs.as_mut_slice()
    }
    pub fn active_tab_data(&self) -> Option<&T> {
        self.tabs.get(self.active_tab)?.as_ref()
    }
    pub fn active_tab_data_mut(&mut self) -> Option<&mut Option<T>> {
        self.tabs.get_mut(self.active_tab)
    }
}
impl<T> TabsView<T> {
    pub fn on_action_close_tab(
        &mut self,
        _: &CloseTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remove_tab(self.active_tab, window, cx);
    }
    pub fn on_action_create_tab(
        &mut self,
        _: &CreateTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.create_tab(None, window, cx);
        self.scroll_to_active_tab(window, cx);
        cx.notify();
    }
    pub fn on_action_next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.set_active_tab(self.active_tab + 1, window, cx);
        self.scroll_to_active_tab(window, cx);
        cx.notify();
    }
    pub fn on_action_prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        self.set_active_tab(self.active_tab.saturating_sub(1), window, cx);
        self.scroll_to_active_tab(window, cx);
        cx.notify();
    }
}

/// Payload for `on_drag` event.
#[derive(Debug, Clone)]
struct DragTab {
    /// Index of the dragged tab.
    index: usize,
    /// Label of the dragged tab.
    label: SharedString,
}
impl Render for DragTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("drag-tab")
            .cursor_grab()
            .py_1()
            .px_3()
            .border_2()
            .whitespace_nowrap()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .text_color(cx.theme().tab_foreground)
            .bg(cx.theme().tab_active)
            .opacity(0.75)
            .child(self.label.clone())
    }
}
impl<T: TabData> Render for TabsView<T> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.smooth_scroll
            .noticed_scroll(window, cx, &self.scroll_handle);
        self.smooth_scroll
            .preform_scroll(window, cx, &self.scroll_handle);

        let new_tab_button = if self.tabs.is_empty() {
            Empty.into_any_element()
        } else {
            div()
                .top_0()
                .right_0()
                .bottom_0()
                .child(
                    Button::new("button-new-tab-pdf")
                        .icon(Icon::new(IconName::Plus))
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.create_tab(None, window, cx);
                            view.scroll_to_active_tab(window, cx);
                        })),
                )
                .flex_none()
                .into_any_element()
        };
        let tab_bar = TabBar::new("dynamic-tabs-with-pdf-files")
            .menu(self.tabs.len() > 1)
            .selected_index(self.active_tab)
            .track_scroll(&self.scroll_handle)
            .on_click(cx.listener(|view, index, window, cx| {
                view.set_active_tab(*index, window, cx);
                view.scroll_to_active_tab(window, cx);
            }))
            .children(self.tabs.iter().enumerate().map(|(tab_index, tab_data)| {
                let label = if let Some(tab_data) = tab_data {
                    tab_data.label()
                } else {
                    "New tab".into()
                };
                Tab::new()
                    .label(label.clone())
                    .rounded(cx.theme().radius)
                    .on_drag(
                        DragTab {
                            index: tab_index,
                            label: label.clone(),
                        },
                        |drag, _, _, cx| {
                            cx.stop_propagation();
                            cx.new(|_| drag.clone())
                        },
                    )
                    .drag_over::<DragTab>(|this, _, _, cx| {
                        this.border_l_2().border_color(cx.theme().drag_border)
                    })
                    .on_drop(cx.listener(move |view, drag: &DragTab, _window, cx| {
                        let tab = view.tabs.remove(drag.index);
                        view.tabs.insert(tab_index, tab);
                        if view.active_tab == drag.index {
                            view.active_tab = tab_index;
                        } else if view.active_tab > drag.index && view.active_tab <= tab_index {
                            view.active_tab -= 1;
                        } else if view.active_tab < drag.index && view.active_tab >= tab_index {
                            view.active_tab += 1;
                        }
                        cx.notify();
                    }))
                    .child(
                        // Non-close button area:
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right_0()
                            .left_0()
                            .on_any_mouse_down(cx.listener(move |view, event, window, cx| {
                                if let MouseDownEvent {
                                    button: MouseButton::Middle,
                                    ..
                                } = event
                                {
                                    view.remove_tab(tab_index, window, cx);
                                }
                            })),
                    )
                    .suffix(
                        Button::new("button-close-tab")
                            .icon(Icon::new(IconName::Close))
                            .on_click(cx.listener(move |view, _event, window, cx| {
                                cx.stop_propagation();
                                view.remove_tab(tab_index, window, cx);
                            }))
                            .max_w_6()
                            .max_h_6(),
                    )
                    .when_some(tab_data.as_ref(), |this, full_path| {
                        this.tooltip({
                            let full_path =
                                SharedString::from(format!("{}", full_path.full_path().display()));
                            move |window, cx| Tooltip::new(full_path.clone()).build(window, cx)
                        })
                    })
            }));

        div()
            .flex()
            .flex_row_reverse()
            .w_full()
            .overflow_hidden()
            .refine_style(&StyleRefinement {
                align_items: Some(AlignItems::Stretch),
                ..Default::default()
            })
            .child(new_tab_button)
            .child(
                div()
                    .flex_1()
                    .left_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .absolute()
                            .bottom_0()
                            .top_0()
                            .right_0()
                            .left_0()
                            .on_scroll_wheel(cx.listener(
                                |view, event: &ScrollWheelEvent, window, cx| {
                                    if event.control {
                                        view.scroll_handle
                                            .set_offset(view.smooth_scroll.wanted_offset());

                                        match event.delta.pixel_delta(px(1.0)).y.cmp(&px(0.)) {
                                            Ordering::Greater => {
                                                view.set_active_tab(
                                                    view.active_tab.saturating_sub(1),
                                                    window,
                                                    cx,
                                                );
                                                view.scroll_to_active_tab(window, cx);
                                            }
                                            Ordering::Equal => {}
                                            Ordering::Less => {
                                                view.set_active_tab(
                                                    view.active_tab.saturating_add(1),
                                                    window,
                                                    cx,
                                                );
                                                view.scroll_to_active_tab(window, cx);
                                            }
                                        }
                                    } else {
                                        view.smooth_scroll.noticed_scroll_wheel_event(
                                            window,
                                            cx,
                                            &view.scroll_handle,
                                        );
                                    }
                                },
                            )),
                    )
                    .child(tab_bar),
            )
    }
}
