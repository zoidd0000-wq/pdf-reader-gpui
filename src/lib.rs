#[cfg(target_family = "wasm")]
extern crate gpui_component_assets_git as gpui_component_assets;
#[cfg(target_family = "wasm")]
extern crate gpui_component_git as gpui_component;
#[cfg(target_family = "wasm")]
extern crate gpui_git as gpui;
#[cfg(target_family = "wasm")]
extern crate gpui_platform_git as gpui_platform;

#[cfg(not(target_family = "wasm"))]
pub mod assets;
pub mod elm;
pub mod pdf;
pub mod prompt;
pub mod session;
pub mod tabs;

use crate::elm::{MsgSender, Update};
use crate::prompt::{NoDisplayHandle, prompt_load_pdf_file};
use crate::tabs::TabsView;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, AsyncWindowContext, Bounds, ClipboardItem, Context, Entity, FocusHandle,
    ImageCacheError, ImageSource, InteractiveElement, IntoElement, KeyBinding, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement, Pixels, Point, Render,
    RenderImage, Resource, ScrollHandle, SharedString, Size, Styled, StyledImage, Task,
    WeakEntity, Window, WindowOptions, canvas, div, hsla, img, px, size,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::scroll::Scrollbar;
use gpui_component::{Root, StyledExt, VirtualListScrollHandle, h_flex, v_flex, v_virtual_list};
use hayro::RenderSettings;
use hayro_interpret::InterpreterSettings;
use hayro_syntax::Pdf;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::PathBuf;
use std::pin::Pin;
use std::rc::Rc;
#[cfg(not(target_family = "wasm"))]
use std::sync::Condvar;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};
use std::time::Duration;
#[cfg(target_family = "wasm")]
use wasm_bindgen::prelude::*;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const SNIP_SCALE: f32 = 2.0;

#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = pdf_reader)]
pub struct ZoomIn;

#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = pdf_reader)]
pub struct ZoomOut;

#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = pdf_reader)]
pub struct ZoomReset;

#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = pdf_reader)]
pub struct ToggleSnip;

#[derive(Clone, Copy, PartialEq)]
pub struct RenderSettings2 {
    pub x_scale: f32,
    pub y_scale: f32,
    pub width: Option<u16>,
    pub height: Option<u16>,
}
impl Default for RenderSettings2 {
    fn default() -> Self {
        RenderSettings::default().into()
    }
}
impl From<RenderSettings> for RenderSettings2 {
    fn from(value: RenderSettings) -> RenderSettings2 {
        <RenderSettings2 as From<&'_ RenderSettings>>::from(&value)
    }
}
impl From<&'_ RenderSettings> for RenderSettings2 {
    fn from(value: &RenderSettings) -> Self {
        Self {
            x_scale: value.x_scale,
            y_scale: value.y_scale,
            width: value.width,
            height: value.height,
        }
    }
}
impl From<RenderSettings2> for RenderSettings {
    fn from(value: RenderSettings2) -> Self {
        Self {
            x_scale: value.x_scale,
            y_scale: value.y_scale,
            width: value.width,
            height: value.height,
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        }
    }
}

pub fn range_is_contiguous(a: Range<usize>, b: Range<usize>) -> bool {
    range_union(a.clone(), b.clone()).len() <= a.len() + b.len()
}
pub fn range_union(a: Range<usize>, b: Range<usize>) -> Range<usize> {
    match (a.len(), b.len()) {
        (0, 0) => 0..0,
        (_, 0) => a,
        (0, _) => b,
        _ => a.start.min(b.start)..a.end.max(b.end),
    }
}
pub fn range_intersection(a: Range<usize>, b: Range<usize>) -> Range<usize> {
    let start = a.start.max(b.start);
    start..a.end.min(b.end).max(start)
}

pub struct PdfTabData {
    path: Arc<PathBuf>,
    pdf_data: Arc<Vec<u8>>,
    scroll: RefCell<VirtualListScrollHandle>,
}
impl tabs::TabData for PdfTabData {
    fn label(&self) -> SharedString {
        if let Some(name) = self.path.file_name() {
            name.to_string_lossy().into_owned().into()
        } else {
            "<invalid path>".into()
        }
    }

    fn full_path(&self) -> Arc<PathBuf> {
        self.path.clone()
    }
}

pub struct NoGpuiImageCache;
impl gpui::ImageCache for NoGpuiImageCache {
    fn load(
        &mut self,
        _resource: &Resource,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        None
    }
}

struct ArcIdentity<T>(Arc<T>);
impl<T> Hash for ArcIdentity<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(Arc::as_ptr(&self.0).addr());
    }
}
impl<T> PartialEq for ArcIdentity<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl<T> Eq for ArcIdentity<T> {}

#[cfg(target_family = "wasm")]
struct AsyncCondvar {
    waker: Mutex<Option<std::task::Waker>>,
}
#[cfg(target_family = "wasm")]
impl AsyncCondvar {
    pub const fn new() -> Self {
        Self {
            waker: Mutex::new(None),
        }
    }
    pub fn wait(&self) -> impl Future<Output = ()> {
        struct WaitFut<'a>(&'a AsyncCondvar, bool);
        impl std::future::Future for WaitFut<'_> {
            type Output = ();

            fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
                let this = self.get_mut();
                if this.1 {
                    return Poll::Ready(());
                }
                this.1 = true;
                let prev = this.0.waker.lock().unwrap().replace(cx.waker().clone());
                if let Some(prev) = prev {
                    prev.wake();
                }
                Poll::Pending
            }
        }
        WaitFut(self, false)
    }
    pub fn notify_all(&self) {
        let mut guard = self.waker.lock().unwrap();
        if let Some(waker) = guard.take() {
            waker.wake();
        }
    }
}

macro_rules! async_on_wasm {
    ($($code:tt)*) => {{
        #[cfg(target_family = "wasm")]
        {
            async move {
                $($code)*
            }
        }
        #[cfg(not(target_family = "wasm"))]
        {
            $($code)*
        }
    }};
}
#[cfg(not(target_family = "wasm"))]
macro_rules! future_on_wasm {
    ($ret_ty:ty) => {
        $ret_ty
    };
}
#[cfg(target_family = "wasm")]
macro_rules! future_on_wasm {
    ($ret_ty:ty) => {
        impl Future<Output = $ret_ty>
    };
}

struct PdfPageCacheMutableState {
    images: Vec<Option<Arc<RenderImage>>>,
    render_settings: RenderSettings2,
    pdf: Option<Arc<Pdf>>,
    wake_future: Option<Waker>,
    requested_pages: Range<usize>,
    acknowledged_pages: Range<usize>,
    should_quit: bool,
}
impl PdfPageCacheMutableState {
    pub fn set_new_pdf(&mut self, pdf: Option<Arc<Pdf>>, render_settings: RenderSettings2) {
        self.images.clear();
        if let Some(pdf) = pdf.as_ref() {
            self.images.resize_with(pdf.pages().len(), || None);
        }
        self.requested_pages = 0..0;
        self.acknowledged_pages = 0..0;
        self.render_settings = render_settings;
        self.pdf = pdf;
    }
}
struct PdfPageCacheSharedState {
    state: Mutex<PdfPageCacheMutableState>,
    #[cfg(not(target_family = "wasm"))]
    wake_worker: Condvar,
    #[cfg(target_family = "wasm")]
    wake_worker: AsyncCondvar,
}
struct PdfPageCache {
    shared: Arc<PdfPageCacheSharedState>,
    _ui_updater: Task<()>,
    pages_this_frame: Range<usize>,
    pages_last_frame: Range<usize>,
    rendered_images: HashSet<ArcIdentity<RenderImage>>,
}
impl Drop for PdfPageCache {
    fn drop(&mut self) {
        let mut guard = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.should_quit = true;
        let waker = guard.wake_future.take();
        drop(guard);
        if let Some(waker) = waker {
            waker.wake();
        }
        self.shared.wake_worker.notify_all();
    }
}
impl PdfPageCache {
    pub fn new(window: &mut Window, cx: &mut Context<PdfPages>) -> Self {
        let shared = Arc::new(PdfPageCacheSharedState {
            state: Mutex::new(PdfPageCacheMutableState {
                images: Vec::with_capacity(256),
                render_settings: RenderSettings2 {
                    x_scale: 1.,
                    y_scale: 1.,
                    ..Default::default()
                },
                pdf: None,
                wake_future: None,
                requested_pages: 0..0,
                acknowledged_pages: 0..0,
                should_quit: false,
            }),
            #[cfg(not(target_family = "wasm"))]
            wake_worker: Condvar::new(),
            #[cfg(target_family = "wasm")]
            wake_worker: AsyncCondvar::new(),
        });
        let this = Self {
            shared: shared.clone(),
            _ui_updater: cx.spawn_in(window, {
                let shared = shared.clone();
                async move |parent, window| Self::foreground_work(shared, parent, window).await
            }),
            pages_this_frame: 0..0,
            pages_last_frame: 0..0,
            rendered_images: Default::default(),
        };

        #[cfg(target_family = "wasm")]
        {
            cx.background_spawn(async move { Self::background_work(shared).await })
                .detach();
        }
        #[cfg(not(target_family = "wasm"))]
        {
            std::thread::Builder::new()
                .name("PDF Rasterizer".to_owned())
                .spawn(move || Self::background_work(shared))
                .expect("failed to spawn background worker thread");
        }

        this
    }

    async fn foreground_work(
        shared: Arc<PdfPageCacheSharedState>,
        parent: WeakEntity<PdfPages>,
        window: &mut AsyncWindowContext,
    ) {
        struct WaitForChange<'a> {
            shared: &'a PdfPageCacheSharedState,
            rendered_images: &'a mut Vec<bool>,
        }
        impl<'a> Future for WaitForChange<'a> {
            type Output = bool;

            fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
                let this = self.get_mut();
                let mut guard = this.shared.state.lock().unwrap();

                this.rendered_images
                    .resize_with(guard.images.len(), || false);

                let mut changed_state = false;
                for (cache, is_cached) in guard.images.iter().zip(this.rendered_images.iter_mut()) {
                    if cache.is_some() != *is_cached {
                        *is_cached = cache.is_some();
                        changed_state = true;
                    }
                }

                if guard.should_quit || changed_state {
                    Poll::Ready(guard.should_quit)
                } else {
                    guard.wake_future = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        }

        let mut rendered_images = Vec::with_capacity(256);
        loop {
            let should_quit = WaitForChange {
                shared: &shared,
                rendered_images: &mut rendered_images,
            }
            .await;
            if should_quit {
                return;
            }

            let result = parent.update(window, |_parent, cx| {
                log::debug!("Notify view about new pdf pages");
                cx.notify();
            });
            if result.is_err() {
                return;
            }
        }
    }

    fn background_work(shared: Arc<PdfPageCacheSharedState>) -> future_on_wasm!(()) {
        async_on_wasm!({
            #[cfg_attr(not(target_family = "wasm"), expect(clippy::never_loop, unused_labels))]
            'acquire_guard: loop {
                let _loop_result = {
                    let mut guard = shared.state.lock().unwrap();
                    loop {
                        if guard.should_quit {
                            return;
                        }

                        let mut index_to_render = None;
                        {
                            let mut wanted_pages = guard.requested_pages.clone();

                            wanted_pages.start = wanted_pages.start.saturating_sub(1);

                            let mut chose_index_distance = usize::MAX;
                            let center =
                                wanted_pages.end.saturating_sub(1 + wanted_pages.len() / 2);

                            let cache_first_image = guard.requested_pages.start <= 1;

                            for (index, image) in guard.images.iter_mut().enumerate() {
                                let should_cache = if index == 0 {
                                    cache_first_image
                                } else {
                                    wanted_pages.contains(&index)
                                };
                                if !should_cache {
                                    *image = None;
                                } else if image.is_none() {
                                    let distance = index.abs_diff(center);
                                    if distance < chose_index_distance {
                                        index_to_render = Some(index);
                                        chose_index_distance = distance;
                                    }
                                }
                            }
                        }

                        log::debug!(
                            "Rasterize page {index_to_render:?}, acknowledged_pages={:?}, requested_pages={:?}",
                            guard.acknowledged_pages.clone(),
                            guard.requested_pages.clone()
                        );
                        guard.acknowledged_pages = guard.requested_pages.clone();

                        if let Some(index) = index_to_render {
                            let Some(pdf) = guard.pdf.clone() else {
                                continue;
                            };
                            let render_settings = guard.render_settings;

                            drop(guard);
                            let new_image = pdf::rasterize_pdf_page(
                                &pdf.pages()[index],
                                &InterpreterSettings::default(),
                                &RenderSettings::from(render_settings),
                            );

                            guard = shared.state.lock().unwrap();
                            if guard.render_settings == render_settings
                                && guard
                                    .pdf
                                    .as_ref()
                                    .is_some_and(|new_pdf| Arc::ptr_eq(&pdf, new_pdf))
                                && let Some(image) = guard.images.get_mut(index)
                            {
                                *image = Some(new_image);
                                log::debug!(
                                    "Rasterize image done, index={index}, acknowledged_pages={:?}, wake_frontend={}",
                                    guard.acknowledged_pages,
                                    guard.wake_future.is_some()
                                );
                                if let Some(waker) = guard.wake_future.take() {
                                    waker.wake();
                                }
                            }
                            continue;
                        }

                        #[cfg(not(target_family = "wasm"))]
                        {
                            guard = shared
                                .wake_worker
                                .wait_while(guard, |state| {
                                    !state.should_quit
                                        && state.acknowledged_pages == state.requested_pages
                                })
                                .unwrap();
                        }

                        #[cfg(target_family = "wasm")]
                        {
                            break;
                        }
                    }
                };

                #[cfg(not(target_family = "wasm"))]
                #[expect(unreachable_code)]
                {
                    match _loop_result {}
                }

                #[cfg(target_family = "wasm")]
                {
                    loop {
                        {
                            let guard = shared.state.lock().unwrap();
                            if guard.should_quit
                                || guard.acknowledged_pages != guard.requested_pages
                            {
                                continue 'acquire_guard;
                            }
                        }
                        shared.wake_worker.wait().await;
                    }
                }
            }
        })
    }

    pub fn clear(&self) {
        self.set_new_pdf(None, RenderSettings2::default());
    }
    pub fn set_new_pdf(&self, pdf: Option<Arc<Pdf>>, render_settings: RenderSettings2) {
        let mut guard = self.shared.state.lock().unwrap();
        guard.set_new_pdf(pdf, render_settings);
    }

    pub fn frame_start(&mut self, window: &mut Window, _cx: &mut Context<PdfPages>) {
        log::trace!(r"PdfPage render started \\//");
        self.pages_last_frame = self.pages_this_frame.clone();
        self.pages_this_frame = 0..0;

        self.rendered_images.retain(|image| {
            if Arc::strong_count(&image.0) == 1 {
                _ = window.drop_image(image.0.clone());
                false
            } else {
                true
            }
        });
    }

    pub fn get_images(
        &mut self,
        visible_range: Range<usize>,
        _window: &mut Window,
        _cx: &mut Context<PdfPages>,
    ) -> Vec<Option<Arc<RenderImage>>> {
        let mut guard = self.shared.state.lock().unwrap();
        let images = if let Some(images) = guard.images.get(visible_range.clone()) {
            images.to_vec()
        } else {
            vec![None; visible_range.len()]
        };

        self.rendered_images.extend(
            images
                .iter()
                .filter_map(|slot| slot.as_ref())
                .map(|image| ArcIdentity(image.clone())),
        );

        if visible_range.start == 0 && visible_range.end == 1 {
            return images;
        }

        if self.pages_this_frame.is_empty()
            || !range_is_contiguous(self.pages_this_frame.clone(), visible_range.clone())
        {
            self.pages_this_frame = visible_range.clone();
        } else {
            self.pages_this_frame =
                range_union(self.pages_this_frame.clone(), visible_range.clone());
        }

        guard.requested_pages =
            range_union(self.pages_this_frame.clone(), self.pages_last_frame.clone());

        if guard.requested_pages != guard.acknowledged_pages {
            if let Some(waker) = guard.wake_future.take() {
                waker.wake();
            }
            drop(guard);
            self.shared.wake_worker.notify_all();
        }

        log::trace!(
            "Rendering pdf pages at visible_range={visible_range:?}, current_images={:?}",
            images
                .iter()
                .map(|image| image.is_some())
                .collect::<Vec<_>>()
        );

        images
    }
}

pub struct PdfPages {
    scroll_handle: VirtualListScrollHandle,
    save_scroll: RefCell<VirtualListScrollHandle>,
    item_sizes: Rc<Vec<Size<Pixels>>>,
    pdf_page_cache: PdfPageCache,
    disabled_cache: Entity<NoGpuiImageCache>,
    zoom: f32,
    snipping: bool,
    snip: Option<(usize, Point<Pixels>, Point<Pixels>)>,
    page_bounds: Rc<RefCell<HashMap<usize, Bounds<Pixels>>>>,
}
impl PdfPages {
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            scroll_handle: VirtualListScrollHandle::from(ScrollHandle::default()),
            save_scroll: RefCell::new(VirtualListScrollHandle::from(ScrollHandle::new())),
            item_sizes: Rc::new(vec![]),
            pdf_page_cache: PdfPageCache::new(window, cx),
            disabled_cache: cx.new(|_cx| NoGpuiImageCache),
            zoom: 1.0,
            snipping: false,
            snip: None,
            page_bounds: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    fn finish_snip(&mut self, cx: &mut Context<Self>) {
        let Some((index, a, b)) = self.snip.take() else {
            return;
        };
        self.snipping = false;
        cx.notify();
        let Some(bounds) = self.page_bounds.borrow().get(&index).copied() else {
            return;
        };
        let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        if w <= 0. || h <= 0. {
            return;
        }
        let rel = |p: Pixels, o: Pixels, s: f32| (f32::from(p - o) / s).clamp(0., 1.);
        let x0 = rel(a.x.min(b.x), bounds.origin.x, w);
        let x1 = rel(a.x.max(b.x), bounds.origin.x, w);
        let y0 = rel(a.y.min(b.y), bounds.origin.y, h);
        let y1 = rel(a.y.max(b.y), bounds.origin.y, h);
        if x1 - x0 < 0.01 || y1 - y0 < 0.01 {
            return;
        }
        let (pdf, mut settings) = {
            let guard = self.pdf_page_cache.shared.state.lock().unwrap();
            (guard.pdf.clone(), guard.render_settings)
        };
        let Some(pdf) = pdf else {
            return;
        };
        if index >= pdf.pages().len() {
            return;
        }
        settings.x_scale *= SNIP_SCALE;
        settings.y_scale *= SNIP_SCALE;
        let Some(png) = pdf::crop_page_png(
            &pdf.pages()[index],
            &InterpreterSettings::default(),
            &RenderSettings::from(settings),
            [x0, y0, x1, y1],
        ) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_image(&gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            png,
        )));
    }
}
impl Render for PdfPages {
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        fn weak_image(image: &Arc<RenderImage>) -> ImageSource {
            let image = Arc::downgrade(image);
            ImageSource::Custom(Arc::new(move |_window, _cx| Some(Ok(image.upgrade()?))))
        }

        self.pdf_page_cache.frame_start(window, cx);

        div()
            .relative()
            .size_full()
            .child(
                v_virtual_list(
                    cx.entity().clone(),
                    "pdf-viewer-pages-list",
                    self.item_sizes.clone(),
                    move |view, visible_range, window, cx| {
                        let snipping = view.snipping;
                        let zoom = view.zoom;
                        let selection = view.snip;
                        let page_bounds = view.page_bounds.clone();
                        let viewport_width = window.viewport_size().width;
                        visible_range
                            .clone()
                            .zip(view.pdf_page_cache.get_images(visible_range, window, cx))
                            .map(|(row_ix, page_image)| {
                                let page = if let Some(page_image) = page_image {
                                    img(weak_image(&page_image))
                                        .object_fit(ObjectFit::Cover)
                                        .when(zoom <= 1., |image| image.max_w(viewport_width))
                                        .image_cache(&view.disabled_cache)
                                        .into_any_element()
                                } else {
                                    div().into_any_element()
                                };
                                div()
                                    .relative()
                                    .size_full()
                                    .child(page)
                                    .when(snipping, |item| {
                                        let bounds_sink = page_bounds.clone();
                                        let origin = page_bounds
                                            .borrow()
                                            .get(&row_ix)
                                            .map(|bounds| bounds.origin)
                                            .unwrap_or_default();
                                        item.cursor_crosshair()
                                            .child(
                                                canvas(
                                                    move |bounds, _, _| {
                                                        bounds_sink.borrow_mut().insert(row_ix, bounds);
                                                    },
                                                    |_, _, _, _| {},
                                                )
                                                .absolute()
                                                .size_full(),
                                            )
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(
                                                    move |view, event: &MouseDownEvent, _window, cx| {
                                                        view.snip =
                                                            Some((row_ix, event.position, event.position));
                                                        cx.notify();
                                                    },
                                                ),
                                            )
                                            .on_mouse_move(cx.listener(
                                                move |view, event: &MouseMoveEvent, _window, cx| {
                                                    if let Some((index, start, _)) = view.snip
                                                        && index == row_ix
                                                    {
                                                        view.snip = Some((index, start, event.position));
                                                        cx.notify();
                                                    }
                                                },
                                            ))
                                            .on_mouse_up(
                                                MouseButton::Left,
                                                cx.listener(|view, _: &MouseUpEvent, _window, cx| {
                                                    view.finish_snip(cx);
                                                }),
                                            )
                                            .on_mouse_up_out(
                                                MouseButton::Left,
                                                cx.listener(|view, _: &MouseUpEvent, _window, cx| {
                                                    view.finish_snip(cx);
                                                }),
                                            )
                                            .when_some(
                                                selection.filter(|selected| selected.0 == row_ix),
                                                |item, (_, a, b)| {
                                                    let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
                                                    let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
                                                    item.child(
                                                        div()
                                                            .absolute()
                                                            .left(x0 - origin.x)
                                                            .top(y0 - origin.y)
                                                            .w(x1 - x0)
                                                            .h(y1 - y0)
                                                            .border_1()
                                                            .border_color(hsla(0.6, 0.9, 0.6, 1.))
                                                            .bg(hsla(0.6, 0.9, 0.6, 0.2)),
                                                    )
                                                },
                                            )
                                    })
                                    .into_any_element()
                            })
                            .collect()
                    },
                )
                .track_scroll(&self.scroll_handle),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .child(Scrollbar::vertical(&self.scroll_handle)),
            )
            .into_any_element()
    }
}

pub struct PdfReader {
    focus_handle: FocusHandle,
    tabs: Entity<TabsView<PdfTabData>>,
    pages: Entity<PdfPages>,
    assumed_viewport_size: Size<Pixels>,
}
impl PdfReader {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("ctrl-w", tabs::CloseTab, Some(CONTEXT)),
            KeyBinding::new("ctrl-t", tabs::CreateTab, Some(CONTEXT)),
            KeyBinding::new("ctrl-tab", tabs::NextTab, Some(CONTEXT)),
            KeyBinding::new("ctrl-shift-tab", tabs::PrevTab, Some(CONTEXT)),
            KeyBinding::new("ctrl-=", ZoomIn, Some(CONTEXT)),
            KeyBinding::new("ctrl--", ZoomOut, Some(CONTEXT)),
            KeyBinding::new("ctrl-0", ZoomReset, Some(CONTEXT)),
            KeyBinding::new("ctrl-shift-s", ToggleSnip, Some(CONTEXT)),
        ]);

        let restored: Vec<PdfTabData> = session::load()
            .into_iter()
            .filter_map(|path| {
                let data = std::fs::read(&path).ok()?;
                Some(PdfTabData {
                    path: Arc::new(path),
                    pdf_data: Arc::new(data),
                    scroll: RefCell::new(VirtualListScrollHandle::from(ScrollHandle::new())),
                })
            })
            .collect();

        Self {
            focus_handle: cx.focus_handle(),
            tabs: {
                let sender = MsgSender::from_cx(window, cx);
                cx.new(|cx| {
                    let mut tabs = TabsView::new(window, cx);
                    tabs.on_tab_changed(move |_window, _cx| {
                        sender
                            .spawn(async move |_window, mut sender| {
                                sender.send(PdfCommand::ChangedTab);
                            })
                            .detach();
                    });
                    let mut restored = restored.into_iter();
                    if let Some(first) = restored.next() {
                        tabs.tabs_data_mut()[0] = Some(first);
                        for data in restored {
                            tabs.create_tab(Some(data), window, cx);
                        }
                        tabs.set_active_tab(0, window, cx);
                    }
                    tabs
                })
            },
            pages: cx.new(|cx| PdfPages::new(window, cx)),
            assumed_viewport_size: Default::default(),
        }
    }

    fn save_session(&self, cx: &App) {
        let paths: Vec<PathBuf> = self
            .tabs
            .read(cx)
            .tabs_data()
            .iter()
            .flatten()
            .map(|tab| tab.path.as_ref().clone())
            .collect();
        session::save(&paths);
    }

    fn zoom_by(&mut self, factor: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.pages.update(cx, |pages, _| {
            pages.zoom = (pages.zoom * factor).clamp(0.25, 4.);
        });
        self.active_pdf_changed(window, cx);
    }

    fn zoom_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pages.update(cx, |pages, _| pages.zoom = 1.);
        self.active_pdf_changed(window, cx);
    }

    fn toggle_snip(&mut self, cx: &mut Context<Self>) {
        self.pages.update(cx, |pages, cx| {
            pages.snipping = !pages.snipping;
            pages.snip = None;
            cx.notify();
        });
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn active_pdf_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pages.update(cx, |pages, cx| {
            pages.item_sizes = Rc::new(vec![]);
            pages.pdf_page_cache.clear();

            *pages.save_scroll.borrow_mut() = pages.scroll_handle.clone();
            pages.scroll_handle = VirtualListScrollHandle::from(ScrollHandle::default());

            let Some(tab_data) = self.tabs.read(cx).active_tab_data() else {
                return;
            };
            pages.scroll_handle = tab_data.scroll.borrow().clone();
            let Ok(pdf) = Pdf::new(tab_data.pdf_data.clone()) else {
                return;
            };
            let pdf: Arc<Pdf> = Arc::new(pdf);
            if pdf.pages().is_empty() {
                return;
            }
            let viewport_size = window.viewport_size();

            let max_width = pdf
                .pages()
                .iter()
                .map(|page| page.render_dimensions().0)
                .max_by(f32::total_cmp)
                .expect("there should be at least one page");
            let viewport_width = f32::from(viewport_size.width);
            let scale = viewport_width / max_width * pages.zoom;

            let render_settings = RenderSettings {
                x_scale: scale,
                y_scale: scale,
                ..Default::default()
            };

            pages
                .pdf_page_cache
                .set_new_pdf(Some(pdf.clone()), render_settings.into());

            self.assumed_viewport_size = viewport_size;
            pages.item_sizes = Rc::new(
                pdf.pages()
                    .iter()
                    .map(|page| {
                        let (width, height) = page.render_dimensions();
                        let (width, height) = (width * scale, height * scale);

                        size(px(width.floor()), px(height.floor()))
                    })
                    .collect::<Vec<_>>(),
            );
        });
    }
    fn check_window_size(&mut self, window: &Window, cx: &mut Context<Self>) {
        let mut latest_window_size = window.viewport_size();
        if self.assumed_viewport_size == Size::default() {
            return;
        }
        if latest_window_size == self.assumed_viewport_size {
            return;
        }
        self.assumed_viewport_size = Size::default();
        let this = cx.weak_entity();

        window
            .spawn(cx, async move |window: &mut AsyncWindowContext| {
                loop {
                    window
                        .background_executor()
                        .timer(Duration::from_millis(250))
                        .await;
                    let keep_checking = window.update(|window, cx| {
                        let new_size = window.viewport_size();
                        if new_size != latest_window_size {
                            latest_window_size = new_size;
                            true
                        } else {
                            _ = this.update(cx, |this, cx| {
                                this.active_pdf_changed(window, cx);
                            });
                            false
                        }
                    });
                    if !matches!(keep_checking, Ok(true)) {
                        break;
                    }
                }
            })
            .detach();
    }
}
const CONTEXT: &str = "pdf-reader";
impl Render for PdfReader {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.check_window_size(window, cx);
        let zoom = self.pages.read(cx).zoom;
        v_flex()
            .size_full()
            .id("pdf-reader")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(window.listener_for(&self.tabs, TabsView::on_action_close_tab))
            .on_action(window.listener_for(&self.tabs, TabsView::on_action_create_tab))
            .on_action(window.listener_for(&self.tabs, TabsView::on_action_next_tab))
            .on_action(window.listener_for(&self.tabs, TabsView::on_action_prev_tab))
            .on_action(cx.listener(|this, _: &ZoomIn, window, cx| this.zoom_by(1.25, window, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, window, cx| this.zoom_by(0.8, window, cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, window, cx| this.zoom_reset(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSnip, _window, cx| this.toggle_snip(cx)))
            .child(self.tabs.clone())
            .child(
                h_flex()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .items_center()
                    .child(
                        Button::new("zoom-out")
                            .label("-")
                            .on_click(cx.listener(|this, _, window, cx| this.zoom_by(0.8, window, cx))),
                    )
                    .child(format!("{}%", (zoom * 100.).round() as u32))
                    .child(
                        Button::new("zoom-in")
                            .label("+")
                            .on_click(cx.listener(|this, _, window, cx| this.zoom_by(1.25, window, cx))),
                    )
                    .child(
                        Button::new("snip")
                            .label("Snip")
                            .on_click(cx.listener(|this, _, _window, cx| this.toggle_snip(cx))),
                    ),
            )
            .child(
                if let Some(tab_data) = self.tabs.read(cx).active_tab_data() {
                    match Pdf::new(tab_data.pdf_data.clone()) {
                        Ok(_) => self.pages.clone().into_any_element(),
                        Err(e) => v_flex()
                            .size_full()
                            .items_center()
                            .justify_center()
                            .child(format!("Failed to load PDF:\n{e:?}"))
                            .into_any_element(),
                    }
                } else {
                    div()
                        .v_flex()
                        .gap_2()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .child(
                            Button::new("ok")
                                .primary()
                                .label("Select a PDF file")
                                .on_click({
                                    let sender = MsgSender::from_cx(window, cx);
                                    move |_, window, _cx| {
                                        let prompt =
                                            prompt_load_pdf_file(Some(&NoDisplayHandle(window)));
                                        sender
                                            .spawn(async move |_window, mut sender| {
                                                if let Some(data) = prompt.await {
                                                    sender.send(PdfCommand::LoadedData(
                                                        #[cfg(not(target_family = "wasm"))]
                                                        data.path().to_owned(),
                                                        #[cfg(target_family = "wasm")]
                                                        data.file_name().into(),
                                                        data.read().await,
                                                    ))
                                                }
                                            })
                                            .detach();
                                    }
                                }),
                        )
                        .into_any_element()
                },
            )
    }
}

pub enum PdfCommand {
    LoadedData(PathBuf, Vec<u8>),
    ChangedTab,
}
impl Update<PdfCommand> for PdfReader {
    fn update(&mut self, window: &mut Window, cx: &mut Context<Self>, msg: PdfCommand) {
        match msg {
            PdfCommand::LoadedData(path, pdf_data) => {
                if let Some(tab_data) = self.tabs.as_mut(cx).active_tab_data_mut() {
                    *tab_data = Some(PdfTabData {
                        path: Arc::new(path),
                        pdf_data: Arc::new(pdf_data),
                        scroll: RefCell::new(VirtualListScrollHandle::from(ScrollHandle::new())),
                    });
                }
                self.active_pdf_changed(window, cx);
                self.save_session(cx);
            }
            PdfCommand::ChangedTab => {
                self.active_pdf_changed(window, cx);
                self.save_session(cx);
            }
        }
    }
}

#[cfg_attr(feature = "hotpath", hotpath::main)]
#[cfg(not(target_family = "wasm"))]
pub fn start_gui() {
    #[cfg(debug_assertions)]
    {
        if std::env::var_os("RUST_LOG").is_none() {
            unsafe { std::env::set_var("RUST_LOG", "trace") };
        }
    }
    env_logger::init();

    gpui::Application::new()
        .with_assets(crate::assets::Assets)
        .run(|cx: &mut App| {
            gpui_component::init(cx);

            cx.open_window(
                WindowOptions {
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("GPUI PDF Reader".into()),
                        ..Default::default()
                    }),
                    window_min_size: Some(Size::new(px(400.), px(400.))),
                    ..Default::default()
                },
                |window: &mut Window, cx: &mut App| {
                    let main_ui = cx.new(|cx: &mut Context<'_, _>| PdfReader::new(window, cx));
                    cx.new(|cx| Root::new(main_ui, window, cx))
                },
            )
            .expect("Failed to build and open window");
        });
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen]
pub fn start_gui() {
    #[cfg(not(debug_assertions))]
    {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            previous(info);
            if let Some(win) = web_sys::window() {
                let _ = win.alert_with_message(&format!(
                    "App panicked and will now stop working:\n{info}"
                ));
            }
        }));
    }

    console_error_panic_hook::set_once();

    console_log::init_with_level(log::Level::Info).expect("Failed to initialize logger");

    tracing_wasm::set_as_global_default();

    let document = web_sys::window()
        .expect("No window")
        .document()
        .expect("No document");

    gpui_platform::web_init();

    gpui_platform::single_threaded_web()
        .with_assets(gpui_component_assets::Assets::new(
            if cfg!(debug_assertions) {
                "http://127.0.0.1:8080/"
            } else {
                "https://lej77.github.io/pdf-reader-gpui/"
            },
        ))
        .run(move |cx| {
            if let Some(loading_text) = document.get_element_by_id("loading") {
                loading_text.remove();
            }

            gpui_component::init(cx);

            cx.open_window(
                WindowOptions::default(),
                |window: &mut Window, cx: &mut App| {
                    let main_ui = cx.new(|cx: &mut Context<'_, _>| PdfReader::new(window, cx));
                    cx.new(|cx| Root::new(main_ui, window, cx))
                },
            )
            .expect("Failed to build and open window");
        });
}
