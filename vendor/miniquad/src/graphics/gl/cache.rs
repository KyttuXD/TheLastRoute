use crate::graphics::*;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct VertexAttributeInternal {
    pub attr_loc: GLuint,
    pub size: i32,
    pub type_: GLuint,
    pub offset: i64,
    pub stride: i32,
    pub buffer_index: usize,
    pub divisor: i32,
    pub gl_pass_as_float: bool,
}

#[derive(Default, Copy, Clone)]
pub struct CachedAttribute {
    pub attribute: VertexAttributeInternal,
    pub gl_vbuf: GLuint,
}

#[derive(Clone, Copy)]
pub struct CachedTexture {
    // GL_TEXTURE_2D or GL_TEXTURE_CUBEMAP
    pub target: GLuint,
    pub texture: GLuint,
}

pub struct GlCache {
    pub stored_index_buffer: GLuint,
    pub stored_index_type: Option<u32>,
    pub stored_vertex_buffer: GLuint,
    pub stored_target: GLuint,
    pub stored_texture: GLuint,
    pub index_buffer: GLuint,
    pub index_type: Option<u32>,
    pub vertex_buffer: GLuint,
    pub textures: [CachedTexture; MAX_SHADERSTAGE_IMAGES],
    pub cur_pipeline: Option<Pipeline>,
    pub cur_pass: Option<RenderPass>,
    pub color_blend: Option<BlendState>,
    pub alpha_blend: Option<BlendState>,
    pub stencil: Option<StencilState>,
    pub color_write: ColorMask,
    pub cull_face: CullFace,
    pub attributes: [Option<CachedAttribute>; MAX_VERTEX_ATTRIBUTES],
}

impl GlCache {
    pub fn bind_buffer(&mut self, target: GLenum, buffer: GLuint, index_type: Option<u32>) {
        if target == GL_ARRAY_BUFFER {
            if self.vertex_buffer != buffer {
                self.vertex_buffer = buffer;
                unsafe {
                    glBindBuffer(target, buffer);
                }
            }
        } else {
            if self.index_buffer != buffer {
                self.index_buffer = buffer;
                unsafe {
                    glBindBuffer(target, buffer);
                }
            }
            self.index_type = index_type;
        }
    }

    pub fn store_buffer_binding(&mut self, target: GLenum) {
        if target == GL_ARRAY_BUFFER {
            self.stored_vertex_buffer = self.vertex_buffer;
        } else {
            self.stored_index_buffer = self.index_buffer;
            self.stored_index_type = self.index_type;
        }
    }

    pub fn restore_buffer_binding(&mut self, target: GLenum) {
        if target == GL_ARRAY_BUFFER {
            if self.stored_vertex_buffer != 0 {
                self.bind_buffer(target, self.stored_vertex_buffer, None);
                self.stored_vertex_buffer = 0;
            }
        } else if self.stored_index_buffer != 0 {
            self.bind_buffer(target, self.stored_index_buffer, self.stored_index_type);
            self.stored_index_buffer = 0;
        }
    }

    pub fn bind_texture(&mut self, slot_index: usize, target: GLuint, texture: GLuint) {
        unsafe {
            glActiveTexture(GL_TEXTURE0 + slot_index as GLuint);
            if self.textures[slot_index].target != target
                || self.textures[slot_index].texture != texture
            {
                let target = if target == 0 { GL_TEXTURE_2D } else { target };
                glBindTexture(target, texture);
                self.textures[slot_index] = CachedTexture { target, texture };
            }
        }
    }

    pub fn store_texture_binding(&mut self, slot_index: usize) {
        self.stored_target = self.textures[slot_index].target;
        self.stored_texture = self.textures[slot_index].texture;
    }

    pub fn restore_texture_binding(&mut self, slot_index: usize) {
        self.bind_texture(slot_index, self.stored_target, self.stored_texture);
    }

    /// Forgets the GL texture name `raw` in the binding cache, as it is about to be deleted (Last Route patch).
    /// Upstream leaves the cache holding a deleted name; the next `store_texture_binding` / `restore_texture_binding`
    /// pair then rebinds it, which WebGL reports as "glBindTexture called with an already deleted texture ID" and
    /// which, on native GL (which reuses names), makes a new texture that got the same name skip its bind. Only the
    /// deleted name is reset, so no GL call is made and live bindings keep their cache entries.
    pub fn forget_texture(&mut self, raw: GLuint) {
        if raw == 0 {
            return;
        }
        for slot in self.textures.iter_mut() {
            if slot.texture == raw {
                *slot = CachedTexture {
                    target: 0,
                    texture: 0,
                };
            }
        }
        if self.stored_texture == raw {
            self.stored_texture = 0;
            self.stored_target = 0;
        }
    }

    pub fn clear_buffer_bindings(&mut self) {
        self.bind_buffer(GL_ARRAY_BUFFER, 0, None);
        self.vertex_buffer = 0;

        self.bind_buffer(GL_ELEMENT_ARRAY_BUFFER, 0, None);
        self.index_buffer = 0;
    }

    pub fn clear_texture_bindings(&mut self) {
        for ix in 0..MAX_SHADERSTAGE_IMAGES {
            if self.textures[ix].texture != 0 {
                self.bind_texture(ix, self.textures[ix].target, 0);
                self.textures[ix] = CachedTexture {
                    target: 0,
                    texture: 0,
                };
            }
        }
    }

    pub fn clear_vertex_attributes(&mut self) {
        for attr_index in 0..MAX_VERTEX_ATTRIBUTES {
            let cached_attr = &mut self.attributes[attr_index];

            if cached_attr.is_some() {
                unsafe { glDisableVertexAttribArray(attr_index as GLuint) };
            }
            *cached_attr = None;
        }
    }
}

// Last Route patch: tests for `forget_texture`. They touch only the cache's fields, never a GL call.
#[cfg(test)]
mod tests {
    use super::*;

    fn cache() -> GlCache {
        GlCache {
            stored_index_buffer: 0,
            stored_index_type: None,
            stored_vertex_buffer: 0,
            index_buffer: 0,
            index_type: None,
            vertex_buffer: 0,
            cur_pipeline: None,
            cur_pass: None,
            color_blend: None,
            alpha_blend: None,
            stencil: None,
            color_write: (true, true, true, true),
            cull_face: CullFace::Nothing,
            stored_texture: 0,
            stored_target: 0,
            textures: [CachedTexture { target: 0, texture: 0 }; MAX_SHADERSTAGE_IMAGES],
            attributes: [None; MAX_VERTEX_ATTRIBUTES],
        }
    }

    #[test]
    fn a_deleted_texture_name_leaves_the_binding_cache() {
        let mut c = cache();
        c.textures[0] = CachedTexture { target: GL_TEXTURE_2D, texture: 17 };
        c.textures[3] = CachedTexture { target: GL_TEXTURE_2D, texture: 17 };
        c.textures[1] = CachedTexture { target: GL_TEXTURE_2D, texture: 9 };
        c.stored_texture = 17;
        c.stored_target = GL_TEXTURE_2D;
        c.forget_texture(17);
        assert_eq!((c.textures[0].texture, c.textures[0].target), (0, 0));
        assert_eq!((c.textures[3].texture, c.textures[3].target), (0, 0));
        assert_eq!(c.textures[1].texture, 9, "another texture's binding stays");
        assert_eq!((c.stored_texture, c.stored_target), (0, 0), "a stored binding of the name is dropped too");
    }

    #[test]
    fn forgetting_leaves_other_stored_bindings_and_name_zero_alone() {
        let mut c = cache();
        c.textures[2] = CachedTexture { target: GL_TEXTURE_2D, texture: 5 };
        c.stored_texture = 5;
        c.stored_target = GL_TEXTURE_2D;
        c.forget_texture(6);
        c.forget_texture(0);
        assert_eq!(c.textures[2].texture, 5);
        assert_eq!(c.stored_texture, 5);
        assert_eq!(c.stored_target, GL_TEXTURE_2D);
        assert_eq!(c.textures[0].texture, 0);
    }
}
