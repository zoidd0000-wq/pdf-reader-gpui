use gpui::RenderImage;
use hayro::vello_cpu::Pixmap;
use hayro::{RenderSettings, render};
use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, FillRule, GlyphDrawMode, Image, InterpreterSettings,
    PageExt, Paint, PathDrawMode, RectExt, SoftMask, interpret,
};
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::object::Object;
use hayro_syntax::page::Page;
use image::{Frame, RgbaImage};
use kurbo::{Affine, BezPath, Point, Rect, Shape};
use std::borrow::Cow;
use std::cell::Cell;
use std::fmt;
use std::fmt::Formatter;
use std::sync::Arc;

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn rasterize_pdf_page(
    page: &Page,
    interpreter_settings: &InterpreterSettings,
    render_settings: &RenderSettings,
) -> Arc<RenderImage> {
    let pixmap = render(page, interpreter_settings, render_settings);
    Arc::new(pixmap_to_gpui_image(pixmap))
}

pub fn crop_page_png(
    page: &Page,
    interpreter_settings: &InterpreterSettings,
    render_settings: &RenderSettings,
    region: [f32; 4],
) -> Option<Vec<u8>> {
    let pixmap = render(page, interpreter_settings, render_settings);
    let width = u32::from(pixmap.width());
    let height = u32::from(pixmap.height());
    let data: Vec<u8> = pixmap
        .take()
        .into_iter()
        .flat_map(|rgba| [rgba.r, rgba.g, rgba.b, rgba.a])
        .collect();
    let full = RgbaImage::from_raw(width, height, data)?;
    let x = ((region[0] * width as f32) as u32).min(width - 1);
    let y = ((region[1] * height as f32) as u32).min(height - 1);
    let w = (((region[2] - region[0]) * width as f32) as u32).clamp(1, width - x);
    let h = (((region[3] - region[1]) * height as f32) as u32).clamp(1, height - y);
    let cropped = image::imageops::crop_imm(&full, x, y, w, h).to_image();
    let mut out = Vec::new();
    cropped
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

pub fn pixmap_to_gpui_image(pixmap: Pixmap) -> RenderImage {
    let width = u32::from(pixmap.width());
    let height = u32::from(pixmap.height());
    let data: Vec<u8> = pixmap
        .take()
        .into_iter()
        .flat_map(|rgba| [rgba.b, rgba.g, rgba.r, rgba.a])
        .collect();

    let image_data = RgbaImage::from_raw(width, height, data).expect("incorrect image dimensions");
    RenderImage::new([Frame::new(image_data)])
}

#[derive(Clone, PartialEq)]
pub enum PdfFeature<'a> {
    Text { text: Cow<'a, [u8]>, rect: Rect },
}
impl PdfFeature<'_> {
    pub fn into_owned(self) -> PdfFeature<'static> {
        match self {
            PdfFeature::Text { text, rect } => PdfFeature::Text {
                text: Cow::Owned(text.into_owned()),
                rect,
            },
        }
    }
}
impl fmt::Debug for PdfFeature<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            PdfFeature::Text { text, rect } => f
                .debug_struct("PdfFeature::Text")
                .field("text", &String::from_utf8_lossy(&text))
                .field("rect", rect)
                .finish(),
        }
    }
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn extract_features(
    page: &Page,
    interpreter_settings: &InterpreterSettings,
    render_settings: &RenderSettings,
    handle_feature: &mut dyn FnMut(PdfFeature<'_>),
) {
    let (x_scale, y_scale) = (render_settings.x_scale, render_settings.y_scale);
    let (width, height) = page.render_dimensions();
    let (scaled_width, scaled_height) = ((width * x_scale) as f64, (height * y_scale) as f64);
    let initial_transform =
        Affine::scale_non_uniform(x_scale as f64, y_scale as f64) * page.initial_transform(true);

    let (pix_width, pix_height) = (
        render_settings.width.unwrap_or(scaled_width.floor() as u16),
        render_settings
            .height
            .unwrap_or(scaled_height.floor() as u16),
    );
    let mut state = Context::new(
        initial_transform,
        Rect::new(0.0, 0.0, pix_width as f64, pix_height as f64),
        page.xref(),
        interpreter_settings.clone(),
    );

    let shared = FeatureExtractorState {
        current_op: Cell::new(None),
        text_region: Cell::new(None),
    };
    let mut device = FeatureExtractor { shared: &shared };

    let mut clip_path = page.intersected_crop_box().to_kurbo().to_path(0.1);
    clip_path.apply_affine(initial_transform);
    device.push_clip_path(&ClipPath {
        path: clip_path,
        fill: FillRule::NonZero,
    });

    let mut data = Vec::new();
    let resources = page.resources();
    let mut ops = page.typed_operations();
    interpret(
        std::iter::from_fn(|| {
            let op = ops.next();
            let prev = shared.current_op.replace(op.clone());
            if let (Some(rect), Some(prev)) = (shared.text_region.take(), prev) {
                data.clear();
                match prev {
                    TypedInstruction::NextLine(_)
                    | TypedInstruction::NextLineAndSetLeading(_)
                    | TypedInstruction::NextLineUsingLeading(_) => {
                        data.push(b'\n');
                    }
                    TypedInstruction::ShowText(text) => {
                        data.extend_from_slice(&text.0.get());
                    }
                    TypedInstruction::NextLineAndShowText(text) => {
                        data.push(b'\n');
                        data.extend_from_slice(&text.0.get());
                    }
                    TypedInstruction::ShowTextWithParameters(text) => {
                        data.push(b'\n');
                        data.extend_from_slice(&text.2.get());
                    }
                    TypedInstruction::ShowTexts(texts) => {
                        for obj in texts.0.iter::<Object>() {
                            if let Some(_adjustment) = obj.clone().into_f32() {
                            } else if let Some(text) = obj.into_string() {
                                data.extend_from_slice(&text.get());
                            }
                        }
                    }
                    _ => log::warn!(
                        "show_glyph used for unexpected PDF operation --- {prev:?} --- {op:?}"
                    ),
                }

                handle_feature(PdfFeature::Text {
                    rect,
                    text: Cow::Borrowed(data.as_slice()),
                });
            }

            op
        }),
        resources,
        &mut state,
        &mut device,
    );

    device.pop_clip_path();
}

struct FeatureExtractorState<'pdf> {
    current_op: Cell<Option<TypedInstruction<'pdf>>>,
    text_region: Cell<Option<Rect>>,
}

struct FeatureExtractor<'out, 'pdf> {
    shared: &'out FeatureExtractorState<'pdf>,
}
impl<'a, 'out, 'pdf> Device<'a> for FeatureExtractor<'out, 'pdf> {
    fn set_soft_mask(&mut self, _mask: Option<SoftMask<'a>>) {}

    fn draw_path(
        &mut self,
        _path: &BezPath,
        _transform: Affine,
        _paint: &Paint<'a>,
        _draw_mode: &PathDrawMode,
    ) {
    }

    fn push_clip_path(&mut self, _clip_path: &ClipPath) {}

    fn push_transparency_group(
        &mut self,
        _opacity: f32,
        _mask: Option<SoftMask<'a>>,
        _blend_mode: BlendMode,
    ) {
    }

    fn draw_glyph(
        &mut self,
        _glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        _paint: &Paint<'a>,
        _draw_mode: &GlyphDrawMode,
    ) {
        let top_left = transform * glyph_transform * Point::new(0., 0.);
        let bottom_right = transform * glyph_transform * Point::new(1., 1.);
        let rect = Rect::from_points(top_left, bottom_right);
        self.shared
            .text_region
            .set(Some(if let Some(prev) = self.shared.text_region.get() {
                rect.union(prev)
            } else {
                rect
            }));
    }

    fn draw_image(&mut self, _image: Image<'a, '_>, _transform: Affine) {}

    fn pop_clip_path(&mut self) {}

    fn pop_transparency_group(&mut self) {}

    fn set_blend_mode(&mut self, _blend_mode: hayro_interpret::BlendMode) {}
}
