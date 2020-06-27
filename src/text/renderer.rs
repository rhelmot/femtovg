
use fnv::FnvHashMap;

use crate::{
    Canvas,
    Renderer,
    Path,
    Paint,
    ErrorKind,
    FillRule,
    ImageId,
    ImageFlags,
    ImageStore,
    PixelFormat,
    ImageInfo,
    RenderTarget,
    Color
};

use super::{
    TextLayout,
    TextStyle,
    RenderStyle,
    Atlas,
    FontDb,
    FontId,
    ShapedGlyph,
};

const GLYPH_PADDING: u32 = 1;
const TEXTURE_SIZE: usize = 512;
const MAX_TEXTURE_SIZE: usize = 4096;

#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub struct RenderedGlyphId {
    glyph_index: u32,
    font_id: FontId,
    size: u16,
    blur: u8,
    render_style: RenderStyle
}

impl RenderedGlyphId {
    fn new(glyph_index: u32, font_id: FontId, style: &TextStyle<'_>) -> Self {
        RenderedGlyphId {
            glyph_index,
            font_id,
            size: style.size,
            blur: style.blur,
            render_style: style.render_style
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct RenderedGlyph {
    texture_index: usize,
    width: u32,
    height: u32,
    atlas_x: u32,
    atlas_y: u32,
    padding: u32,
}

#[derive(Clone)]
pub struct DrawCmd {
    pub image_id: ImageId,
    pub quads: Vec<Quad>
}

#[derive(Copy, Clone, Default, Debug)]
pub struct Quad {
    pub x0: f32,
    pub y0: f32,
    pub s0: f32,
    pub t0: f32,
    pub x1: f32,
    pub y1: f32,
    pub s1: f32,
    pub t1: f32
}

pub struct FontTexture {
    atlas: Atlas,
    image_id: ImageId
}

#[derive(Default)]
pub struct TextRendererContext {
    textures: Vec<FontTexture>,
    glyph_cache: FnvHashMap<RenderedGlyphId, RenderedGlyph>
}

pub fn render_atlas<T: Renderer>(canvas: &mut Canvas<T>, text_layout: &TextLayout, style: &TextStyle<'_>) -> Result<Vec<DrawCmd>, ErrorKind> {
    let mut cmd_map = FnvHashMap::default();

    let half_line_width = if let RenderStyle::Stroke { width } = style.render_style {
        width as f32 / 2.0
    } else {
        0.0
    };

    let initial_render_target = canvas.current_render_target;

    for glyph in &text_layout.glyphs {
        let id = RenderedGlyphId::new(glyph.codepoint, glyph.font_id, style);

        if !canvas.text_renderer_context.glyph_cache.contains_key(&id) {
            let glyph = render_glyph(canvas, style, &glyph)?;

            canvas.text_renderer_context.glyph_cache.insert(id, glyph);
        }

        let rendered = canvas.text_renderer_context.glyph_cache.get(&id).unwrap();

        if let Some(texture) = canvas.text_renderer_context.textures.get(rendered.texture_index) {
            let image_id = texture.image_id;
            let size = texture.atlas.size();
            let itw = 1.0 / size.0 as f32;
            let ith = 1.0 / size.1 as f32;

            let cmd = cmd_map.entry(rendered.texture_index).or_insert_with(|| DrawCmd {
                image_id,
                quads: Vec::new()
            });

            let mut q = Quad::default();

            q.x0 = glyph.x - half_line_width - GLYPH_PADDING as f32;
            q.y0 = glyph.y - half_line_width - GLYPH_PADDING as f32;
            q.x1 = q.x0 + rendered.width as f32;
            q.y1 = q.y0 + rendered.height as f32;

            q.s0 = rendered.atlas_x as f32 * itw;
            q.t0 = rendered.atlas_y as f32 * ith;
            q.s1 = (rendered.atlas_x + rendered.width) as f32 * itw;
            q.t1 = (rendered.atlas_y + rendered.height) as f32 * ith;

            cmd.quads.push(q);
        }
    }

    //dbg!("aasd");

    canvas.set_render_target(initial_render_target);

    // debug draw
    // {
    //     canvas.save();
    //     canvas.reset();

    //     let image_id = canvas.text_renderer_context.textures[0].image_id;

    //     let mut path = Path::new();
    //     path.rect(400.5, 20.5, 512.0, 512.0);
    //     canvas.fill_path(&mut path, Paint::image(image_id, 400.5, 20.5, 512.0, 512.0, 0.0, 1.0));
    //     canvas.stroke_path(&mut path, Paint::color(Color::black()));

    //     canvas.restore();
    // }

    Ok(cmd_map.drain().map(|(_, cmd)| cmd).collect())
}

fn render_glyph<T: Renderer>(
    canvas: &mut Canvas<T>,
    style: &TextStyle<'_>,
    glyph: &ShapedGlyph
) -> Result<RenderedGlyph, ErrorKind> {
    // TODO: this may be blur * 2 - fix it when blurring iss implemented
    let padding = GLYPH_PADDING + style.blur as u32;

    let line_width = if let RenderStyle::Stroke { width } = style.render_style {
        width as u32
    } else {
        0
    };

    let width = glyph.width.ceil() as u32 + line_width + padding * 2;
    let height = glyph.height.ceil() as u32 + line_width + padding * 2;

    let (dst_index, dst_image_id, (dst_x, dst_y)) = find_texture_or_alloc(
        &mut canvas.text_renderer_context.textures,
        &mut canvas.images,
        &mut canvas.renderer,
        width as usize,
        height as usize
    )?;

    // render glyph to image
    canvas.save();
    canvas.reset();

    let (mut path, scale) = {
        let font = canvas.fontdb.get_mut(glyph.font_id).ok_or(ErrorKind::NoFontFound)?;
        let scale = font.scale(style.size as f32);

        let font = font.font_ref();

        let mut path = Path::new();
        font.outline_glyph(owned_ttf_parser::GlyphId(glyph.codepoint as u16), &mut path);

        (path, scale)
    };

    let x = dst_x as f32 - glyph.bearing_x + (line_width as f32 / 2.0) + padding as f32;
    let y = 512.0 - dst_y as f32 - glyph.bearing_y - (line_width as f32 / 2.0) - padding as f32;

    canvas.translate(x, y);
    

    canvas.set_render_target(RenderTarget::Image(dst_image_id));
    canvas.clear_rect(dst_x as u32, 512 - dst_y as u32 - height as u32, width as u32, height as u32, Color::black());

    let factor = 1.0/8.0;

    let mut paint = Paint::color(Color::rgbf(factor, factor, factor));
    paint.set_fill_rule(FillRule::EvenOdd);
    paint.set_anti_alias(false);

    if let RenderStyle::Stroke { width } = style.render_style {
        paint.set_stroke_width(width as f32 / scale);
    }

    canvas.global_composite_blend_func(crate::BlendFactor::SrcAlpha, crate::BlendFactor::One);

    // 4x
    // let points = [
    //     (-3.0/8.0, 1.0/8.0),
    //     (1.0/8.0, 3.0/8.0),
    //     (3.0/8.0, -1.0/8.0),
    //     (-1.0/8.0, -3.0/8.0),
    // ];

    // 8x
    let points = [
        (-7.0/16.0,-1.0/16.0),
        (-1.0/16.0,-5.0/16.0),
        ( 3.0/16.0,-7.0/16.0),
        ( 5.0/16.0,-3.0/16.0),
        ( 5.0/16.0, 1.0/16.0),
        ( 1.0/16.0, 5.0/16.0),
        (-3.0/16.0, 7.0/16.0),
        (-5.0/16.0, 3.0/16.0),
    ];

    for point in &points {
        canvas.save();
        canvas.translate(point.0, point.1);

        canvas.scale(scale, scale);

        if let RenderStyle::Stroke { width: _ } = style.render_style {
            canvas.stroke_path(&mut path, paint);
        } else {
            canvas.fill_path(&mut path, paint);
        }

        canvas.restore();
    }

    canvas.restore();

    if style.blur > 0 {
        // canvas.renderer.blur(
        //     canvas.images.get_mut(dst_image_id).unwrap(),
        //     style.blur,
        //     dst_x + style.blur as usize,
        //     dst_y + style.blur as usize,
        //     width as usize - style.blur as usize,
        //     height as usize - style.blur as usize,
        // );
    }

    Ok(RenderedGlyph {
        width: width,
        height: height,
        atlas_x: dst_x as u32,
        atlas_y: dst_y as u32,
        texture_index: dst_index,
        padding: padding,
    })
}

fn find_texture_or_alloc<T: Renderer>(
    textures: &mut Vec<FontTexture>, 
    images: &mut ImageStore<T::Image>, 
    renderer: &mut T, 
    width: usize, 
    height: usize
) -> Result<(usize, ImageId, (usize, usize)), ErrorKind> {

    // Find a free location in one of the the atlases
    let mut texture_search_result = textures.iter_mut().enumerate().find_map(|(index, texture)| {
        texture.atlas.add_rect(width, height).map(|loc| (index, texture.image_id, loc))
    });

    if texture_search_result.is_none() {
        // All atlases are exausted and a new one must be created
        let mut atlas_size = TEXTURE_SIZE;

        // Try incrementally larger atlasses until a large enough one
        // is found or the MAX_TEXTURE_SIZE limit is reached
        let (atlas, loc) = loop {
            let mut test_atlas = Atlas::new(atlas_size, atlas_size);

            if let Some(loc) = test_atlas.add_rect(width as usize, height as usize) {
                break (test_atlas, Some(loc));
            }

            if atlas_size >= MAX_TEXTURE_SIZE {
                break (test_atlas, None);
            }

            atlas_size *= 2;
        };

        let loc = loc.ok_or(ErrorKind::FontSizeTooLargeForAtlas)?;

        let info = ImageInfo::new(ImageFlags::empty(), atlas.size().0, atlas.size().1, PixelFormat::Gray8);
        let image_id = images.alloc(renderer, info)?;

        textures.push(FontTexture { atlas, image_id });

        let index = textures.len() - 1;
        texture_search_result = Some((index, image_id, loc));
    }

    texture_search_result.ok_or(ErrorKind::UnknownError)
}

pub fn render_direct<T: Renderer>(canvas: &mut Canvas<T>, text_layout: &TextLayout, style: &TextStyle<'_>, paint: &Paint, invscale: f32) -> Result<(), ErrorKind> {
    let mut paint = *paint;
    paint.set_fill_rule(FillRule::EvenOdd);
    //paint.set_anti_alias(false);

    //canvas.save();
    //canvas.set_global_alpha(0.25);
    //canvas.global_composite_blend_func(crate::BlendFactor::SrcAlpha, crate::BlendFactor::One);

    // let points = [
    //     (-3.0/8.0, 1.0/8.0),
    //     (1.0/8.0, 3.0/8.0),
    //     (3.0/8.0, -1.0/8.0),
    //     (-1.0/8.0, -3.0/8.0),
    // ];

    for glyph in &text_layout.glyphs {

        let (mut path, scale) = {
            let font = canvas.fontdb.get_mut(glyph.font_id).ok_or(ErrorKind::NoFontFound)?;
            let scale = font.scale(style.size as f32);
            let font = font.font_ref();

            //let mut transform = Transform2D::identity();

            //transform.scale(scale * invscale, -scale * invscale);
            //transform.translate((glyph.x - glyph.bearing_x) * invscale, (glyph.y + glyph.bearing_y) * invscale);
            
            //let mut path_builder = GlyphPathBuilder(Path::new(), transform);
            let mut path = Path::new();
            font.outline_glyph(owned_ttf_parser::GlyphId(glyph.codepoint as u16), &mut path);

            (path, scale)
        };

        canvas.save();

        if let RenderStyle::Stroke { width } = style.render_style {
            paint.set_stroke_width(width as f32 / scale);
        }

        canvas.translate((glyph.x - glyph.bearing_x) * invscale, (glyph.y + glyph.bearing_y) * invscale);
        canvas.scale(scale * invscale, -scale * invscale);

        if let RenderStyle::Stroke { width:_ } = style.render_style {
            canvas.stroke_path(&mut path, paint);
        } else {
            canvas.fill_path(&mut path, paint);
        }

        canvas.restore();

        // for point in &points {
        //     canvas.save();
        //     canvas.translate(point.0, point.1);
    
        //     if let RenderStyle::Stroke { width: _ } = style.render_style {
        //         canvas.stroke_path(&mut path, paint);
        //     } else {
        //         canvas.fill_path(&mut path, paint);
        //     }
    
        //     canvas.restore();
        // }
    }

    //canvas.restore();

    Ok(())
}