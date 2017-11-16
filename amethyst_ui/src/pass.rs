//! Simple flat forward drawing pass.

use std::cmp::{Ordering, PartialOrd};
use std::hash::{Hash, Hasher};

use amethyst_assets::{AssetStorage, Loader};
use amethyst_core::Time;
use amethyst_renderer::{Encoder, Factory, Mesh, MeshHandle, PosTex, Resources, ScreenDimensions,
                        Texture, TextureData, TextureHandle, TextureMetadata, VertexFormat};
use amethyst_renderer::error::Result;
use amethyst_renderer::pipe::{Effect, NewEffect};
use amethyst_renderer::pipe::pass::{Pass, PassData};
use cgmath::vec4;
use fnv::FnvHashMap as HashMap;
use gfx::preset::blend;
use gfx::pso::buffer::ElemStride;
use gfx::state::ColorMask;
use gfx_glyph::{BuiltInLineBreaker, FontId, GlyphBrush, GlyphBrushBuilder, HorizontalAlign,
                Layout, Scale, SectionText, VariedSection, VerticalAlign};
use hibitset::BitSet;
use specs::{Entities, Entity, Fetch, Join, ReadStorage, WriteStorage};
use unicode_segmentation::UnicodeSegmentation;

use super::*;

const VERT_SRC: &[u8] = include_bytes!("shaders/vertex.glsl");
const FRAG_SRC: &[u8] = include_bytes!("shaders/frag.glsl");

#[derive(Copy, Clone, Debug)]
#[allow(dead_code)] // This is used by the shaders
#[repr(C)]
struct VertexArgs {
    proj_vec: [f32; 4],
    coord: [f32; 2],
    dimension: [f32; 2],
}

#[derive(Clone, Debug)]
struct CachedDrawOrder {
    pub cached: BitSet,
    pub cache: Vec<(f32, Entity)>,
}

/// A color used to query a hashmap for a cached texture of that color.
struct KeyColor(pub [u8; 4]);

impl Eq for KeyColor {}

impl PartialEq for KeyColor {
    fn eq(&self, other: &Self) -> bool {
        self.0[0] == other.0[0] && self.0[1] == other.0[1] && self.0[2] == other.0[2]
            && self.0[3] == other.0[3]
    }
}

impl Hash for KeyColor {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        Hash::hash_slice(&self.0, hasher);
    }
}

/// Draw Ui elements.  UI won't display without this.  Ir's recommended this be your last pass.
pub struct DrawUi {
    mesh_handle: MeshHandle,
    cached_draw_order: CachedDrawOrder,
    cached_color_textures: HashMap<KeyColor, TextureHandle>,
    glyph_brushes: Vec<GlyphBrush<'static, Resources, Factory>>,
}

impl DrawUi {
    /// Create instance of `DrawUi` pass
    pub fn new(loader: &Loader, mesh_storage: &AssetStorage<Mesh>) -> Self {
        // Initialize a single unit quad, we'll use this mesh when drawing quads later
        let data = vec![
            PosTex {
                position: [0., 1., 0.],
                tex_coord: [0., 0.],
            },
            PosTex {
                position: [1., 1., 0.],
                tex_coord: [1., 0.],
            },
            PosTex {
                position: [1., 0., 0.],
                tex_coord: [1., 1.],
            },
            PosTex {
                position: [0., 1., 0.],
                tex_coord: [0., 0.],
            },
            PosTex {
                position: [1., 0., 0.],
                tex_coord: [1., 1.],
            },
            PosTex {
                position: [0., 0., 0.],
                tex_coord: [0., 1.],
            },
        ].into();
        let mesh_handle = loader.load_from_data(data, (), mesh_storage);
        DrawUi {
            mesh_handle,
            cached_draw_order: CachedDrawOrder {
                cached: BitSet::new(),
                cache: Vec::new(),
            },
            cached_color_textures: HashMap::default(),
            glyph_brushes: Vec::new(),
        }
    }
}

impl<'a> PassData<'a> for DrawUi {
    type Data = (
        Entities<'a>,
        Fetch<'a, Loader>,
        Fetch<'a, Time>,
        Fetch<'a, ScreenDimensions>,
        Fetch<'a, AssetStorage<Mesh>>,
        Fetch<'a, AssetStorage<Texture>>,
        Fetch<'a, AssetStorage<FontAsset>>,
        ReadStorage<'a, UiImage>,
        ReadStorage<'a, UiTransform>,
        WriteStorage<'a, UiText>,
        ReadStorage<'a, TextEditing>,
    );
}

impl Pass for DrawUi {
    fn compile(&self, effect: NewEffect) -> Result<Effect> {
        use std::mem;
        effect
            .simple(VERT_SRC, FRAG_SRC)
            .with_raw_constant_buffer("VertexArgs", mem::size_of::<VertexArgs>(), 1)
            .with_raw_vertex_buffer(PosTex::ATTRIBUTES, PosTex::size() as ElemStride, 0)
            .with_texture("albedo")
            .with_blended_output("color", ColorMask::all(), blend::ALPHA, None)
            .build()
    }

    fn apply<'a, 'b: 'a>(
        &'a mut self,
        encoder: &mut Encoder,
        effect: &mut Effect,
        factory: Factory,
        (
            entities,
            loader,
            time,
            screen_dimensions,
            mesh_storage,
            tex_storage,
            font_storage,
            ui_image,
            ui_transform,
            mut ui_text,
            editing,
        ): (
            Entities<'a>,
            Fetch<'a, Loader>,
            Fetch<'a, Time>,
            Fetch<'a, ScreenDimensions>,
            Fetch<'a, AssetStorage<Mesh>>,
            Fetch<'a, AssetStorage<Texture>>,
            Fetch<'a, AssetStorage<FontAsset>>,
            ReadStorage<'a, UiImage>,
            ReadStorage<'a, UiTransform>,
            WriteStorage<'a, UiText>,
            ReadStorage<'a, TextEditing>,
        ),
    ) {
        // Populate and update the draw order cache.
        {
            let bitset = &mut self.cached_draw_order.cached;
            self.cached_draw_order.cache.retain(|&(_z, entity)| {
                let keep = ui_transform.get(entity).is_some();
                if !keep {
                    bitset.remove(entity.id());
                }
                keep
            });
        }


        for &mut (ref mut z, entity) in &mut self.cached_draw_order.cache {
            *z = ui_transform.get(entity).unwrap().z;
        }

        // Attempt to insert the new entities in sorted position.  Should reduce work during
        // the sorting step.
        let transform_set = ui_transform.check();
        {
            // Create a bitset containing only the new indices.
            let new = (&transform_set ^ &self.cached_draw_order.cached) & &transform_set;
            for (entity, transform, _new) in (&*entities, &ui_transform, &new).join() {
                let pos = self.cached_draw_order
                    .cache
                    .iter()
                    .position(|&(cached_z, _)| transform.z >= cached_z);
                match pos {
                    Some(pos) => self.cached_draw_order
                        .cache
                        .insert(pos, (transform.z, entity)),
                    None => self.cached_draw_order.cache.push((transform.z, entity)),
                }
            }
        }
        self.cached_draw_order.cached = transform_set;

        // Sort from largest z value to smallest z value.
        // Most of the time this shouldn't do anything but you still need it for if the z values
        // change.
        self.cached_draw_order
            .cache
            .sort_unstable_by(|&(z1, _), &(z2, _)| {
                z2.partial_cmp(&z1).unwrap_or(Ordering::Equal)
            });

        //let cached_draw_order = &cached_draw_order;

        let proj_vec = vec4(
            2. / screen_dimensions.width(),
            -2. / screen_dimensions.height(),
            -2.,
            1.,
        );

        let mesh = match mesh_storage.get(&self.mesh_handle) {
            Some(mesh) => mesh,
            None => return,
        };

        let vbuf = match mesh.buffer(PosTex::ATTRIBUTES) {
            Some(vbuf) => vbuf.clone(),
            None => return,
        };
        effect.data.vertex_bufs.push(vbuf);
        for &(_z, entity) in &self.cached_draw_order.cache {
            // This won't panic as we guaranteed earlier these entities are present.
            let ui_transform = ui_transform.get(entity).unwrap();
            let vertex_args = VertexArgs {
                proj_vec: proj_vec.into(),
                coord: [ui_transform.x, ui_transform.y],
                dimension: [ui_transform.width, ui_transform.height],
            };
            effect.update_constant_buffer("VertexArgs", &vertex_args, encoder);
            if let Some(image) = ui_image
                .get(entity)
                .and_then(|image| tex_storage.get(&image.texture))
            {
                effect.data.textures.push(image.view().clone());
                effect.data.samplers.push(image.sampler().clone());
                effect.draw(mesh.slice(), encoder);
                effect.data.textures.clear();
                effect.data.samplers.clear();
            }

            if let Some(ui_text) = ui_text.get_mut(entity) {
                // Maintain glyph brushes.
                if ui_text.brush_id.is_none() || ui_text.font != ui_text.cached_font {
                    let font = match font_storage.get(&ui_text.font) {
                        Some(font) => font,
                        None => continue,
                    };
                    let mut new_id = None;

                    // This needs a replacement since it's not possible to implement Eq for Font
                    // in a reasonable manner.
                    /*self.glyph_brushes.iter().position(|brush| {
                        // GlyphBrush guarantees font 0 will be available.
                        brush.fonts().get(&FontId(0)).unwrap() == font
                    });*/
                    if new_id.is_none() {
                        new_id = Some(self.glyph_brushes.len());
                        self.glyph_brushes.push(
                            GlyphBrushBuilder::using_font(font.0.clone()).build(factory.clone()),
                        );
                    }
                    ui_text.brush_id = new_id;
                    ui_text.cached_font = ui_text.font.clone();
                }
                // Build text sections.
                let editing = editing.get(entity);
                let text = editing
                    .and_then(|editing| {
                        if editing.highlight_vector == 0 {
                            return None;
                        }
                        let start = editing
                            .cursor_position
                            .min(editing.cursor_position + editing.highlight_vector)
                            as usize;
                        let end = editing
                            .cursor_position
                            .max(editing.cursor_position + editing.highlight_vector)
                            as usize;
                        let start_byte =
                            ui_text.text.grapheme_indices(true).nth(start).map(|i| i.0);
                        let end_byte = ui_text.text.grapheme_indices(true).nth(end).map(|i| i.0)
                            .unwrap_or(ui_text.text.len());
                        start_byte
                            .map(|start_byte| (editing, (start_byte, end_byte)))
                    })
                    .map(|(editing, (start_byte, end_byte))| {
                        vec![
                            SectionText {
                                text: &((&ui_text.text)[0..start_byte]),
                                scale: Scale::uniform(ui_text.font_size),
                                color: ui_text.color,
                                font_id: FontId(0),
                            },
                            SectionText {
                                text: &((&ui_text.text)[start_byte..end_byte]),
                                scale: Scale::uniform(ui_text.font_size),
                                color: editing.selected_text_color,
                                font_id: FontId(0),
                            },
                            SectionText {
                                text: &((&ui_text.text)[end_byte..]),
                                scale: Scale::uniform(ui_text.font_size),
                                color: ui_text.color,
                                font_id: FontId(0),
                            },
                        ]
                    })
                    .unwrap_or(vec![
                        SectionText {
                            text: &ui_text.text,
                            scale: Scale::uniform(ui_text.font_size),
                            color: ui_text.color,
                            font_id: FontId(0),
                        },
                    ]);
                let layout = Layout::Wrap {
                    line_breaker: BuiltInLineBreaker::UnicodeLineBreaker,
                    h_align: HorizontalAlign::Left,
                    v_align: VerticalAlign::Top,
                };
                let section = VariedSection {
                    screen_position: (ui_transform.x, ui_transform.y),
                    bounds: (ui_transform.width, ui_transform.height),
                    z: ui_transform.z,
                    layout,
                    text,
                };
                // Render background highlight
                let brush = &mut self.glyph_brushes[ui_text.brush_id.unwrap()];
                let cache = &mut self.cached_color_textures;
                if let Some((texture, (start, end))) = editing.and_then(|ed| {
                    let start = ed
                        .cursor_position
                        .min(ed.cursor_position + ed.highlight_vector)
                        as usize;
                    let end = ed
                        .cursor_position
                        .max(ed.cursor_position + ed.highlight_vector)
                        as usize;
                    tex_storage.get(&cached_color_texture(
                        cache,
                        ed.selected_background_color,
                        &loader,
                        &tex_storage,
                    )).map(|tex| (tex, (start, end)))
                }) {
                    effect.data.textures.push(texture.view().clone());
                    effect.data.samplers.push(texture.sampler().clone());
                    let ascent = brush
                        .fonts()
                        .get(&FontId(0))
                        .unwrap()
                        .v_metrics(Scale::uniform(ui_text.font_size))
                        .ascent;
                    for glyph in brush
                        .glyphs(&section)
                        .enumerate()
                        .filter(|&(i, _g)| start <= i && i < end)
                        .map(|(_i, g)| g)
                    {
                        let height = glyph.scale().y;
                        let width = glyph.unpositioned().h_metrics().advance_width;
                        let pos = glyph.position();
                        let vertex_args = VertexArgs {
                            proj_vec: proj_vec.into(),
                            coord: [pos.x, pos.y - ascent],
                            dimension: [width, height],
                        };
                        effect.update_constant_buffer("VertexArgs", &vertex_args, encoder);
                        effect.draw(mesh.slice(), encoder);
                    }
                    effect.data.textures.clear();
                    effect.data.samplers.clear();
                }
                // Render text
                brush.queue(section.clone());
                if let Err(err) = brush.draw_queued(
                    encoder,
                    &effect.data.out_blends[0],
                    &effect.data.out_depth.as_ref().unwrap().0,
                ) {
                    eprintln!("Unable to draw text! Error: {:?}", err);
                }
                // Render cursor
                if let Some((texture, editing)) = editing.as_ref().and_then(|ed| {
                    tex_storage
                        .get(&cached_color_texture(
                            cache,
                            ui_text.color,
                            &loader,
                            &tex_storage,
                        ))
                        .map(|tex| (tex, ed))
                }) {
                    let blink_on = time.total_game_time_seconds() % (1.0 / CURSOR_BLINK_RATE)
                        < 0.5 / CURSOR_BLINK_RATE;
                    if editing.use_block_cursor || blink_on {
                        effect.data.textures.push(texture.view().clone());
                        effect.data.samplers.push(texture.sampler().clone());
                        let ascent = brush
                            .fonts()
                            .get(&FontId(0))
                            .unwrap()
                            .v_metrics(Scale::uniform(ui_text.font_size))
                            .ascent;
                        let glyph_len = brush.glyphs(&section).count();
                        let glyph = if editing.cursor_position as usize >= glyph_len {
                            (brush.glyphs(&section).last(), true)
                        } else {
                            (brush.glyphs(&section).nth(editing.cursor_position as usize), false)
                        };
                        if let (Some(glyph), at_end) = glyph
                        {
                            let height;
                            let width;
                            if editing.use_block_cursor {
                                height = if blink_on {
                                    glyph.scale().y
                                } else {
                                    glyph.scale().y / 10.0
                                };
                                width = glyph.unpositioned().h_metrics().advance_width;
                            } else {
                                height = glyph.scale().y;
                                width = 1.0;
                            }
                            let pos = glyph.position();
                            let mut x = pos.x;
                            if at_end {
                                x += glyph.unpositioned().h_metrics().advance_width;
                            }
                            let mut y = pos.y - ascent;
                            if editing.use_block_cursor && !blink_on {
                                y += glyph.scale().y * 0.9;
                            }
                            let vertex_args = VertexArgs {
                                proj_vec: proj_vec.into(),
                                coord: [x, y],
                                dimension: [width, height],
                            };
                            effect.update_constant_buffer("VertexArgs", &vertex_args, encoder);
                            effect.draw(mesh.slice(), encoder);
                        }
                        effect.data.textures.clear();
                        effect.data.samplers.clear();
                    }
                }
            }
        }
    }
}

fn cached_color_texture(
    cache: &mut HashMap<KeyColor, TextureHandle>,
    color: [f32; 4],
    loader: &Loader,
    storage: &AssetStorage<Texture>,
) -> TextureHandle {
    fn to_u8(input: f32) -> u8 {
        (input * 255.0).min(255.0) as u8
    }
    let key = KeyColor([
        to_u8(color[0]),
        to_u8(color[1]),
        to_u8(color[2]),
        to_u8(color[3]),
    ]);
    cache
        .entry(key)
        .or_insert_with(|| {
            let meta = TextureMetadata {
                sampler: None,
                mip_levels: Some(1),
                size: Some((1, 1)),
                dynamic: false,
                format: None,
                channel: None,
            };
            let texture_data = TextureData::Rgba(color, meta);
            loader.load_from_data(texture_data, (), storage)
        })
        .clone()
}
