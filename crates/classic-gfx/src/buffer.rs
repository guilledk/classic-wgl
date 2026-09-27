//! Static and dynamic vertex buffers, the shared quad, and attrib helpers.

use glow::HasContext;

// ---------------------------------------------------------------------------
// Buffer
// ---------------------------------------------------------------------------

pub struct GlBuffer {
    buffer: glow::Buffer,
    target: u32,
    #[allow(dead_code)]
    count: usize,
}

impl GlBuffer {
    pub fn from_slice<T: bytemuck::Pod>(
        gl: &glow::Context,
        target: u32,
        data: &[T],
        usage: u32,
    ) -> Self {
        let buffer = unsafe { gl.create_buffer() }.expect("create buffer");
        let bytes: &[u8] = bytemuck::cast_slice(data);
        unsafe {
            gl.bind_buffer(target, Some(buffer));
            gl.buffer_data_u8_slice(target, bytes, usage);
        }
        Self { buffer, target, count: data.len() }
    }

    pub fn bind(&self, gl: &glow::Context) {
        unsafe { gl.bind_buffer(self.target, Some(self.buffer)) }
    }

    pub fn sub_data<T: bytemuck::Pod>(&self, gl: &glow::Context, data: &[T]) {
        let bytes: &[u8] = bytemuck::cast_slice(data);
        self.bind(gl);
        unsafe { gl.buffer_sub_data_u8_slice(self.target, 0, bytes) }
    }

    /// Bind this buffer to a numbered indexed-buffer binding point (used for
    /// uniform blocks; the target must be `UNIFORM_BUFFER`).
    pub fn bind_base(&self, gl: &glow::Context, index: u32) {
        unsafe { gl.bind_buffer_base(self.target, index, Some(self.buffer)) }
    }
}

// ---------------------------------------------------------------------------
// 3D model meshes (glTF node-hierarchy path)
// ---------------------------------------------------------------------------

/// Vertex stride of a model mesh VBO: `pos(3) + normal(3) + uv(2)` = 8 `f32`.
pub const MODEL_VERTEX_STRIDE: i32 = 32;

/// A GPU-resident model mesh: interleaved `[position, normal, uv]` vertices +
/// `u32` triangle indices (WebGL2 supports `UNSIGNED_INT` elements natively).
pub struct ModelMeshGpu {
    pub vbo: GlBuffer,
    pub indices: GlBuffer,
    pub index_count: usize,
}

impl ModelMeshGpu {
    /// Upload a mesh.  Missing normals default to `+Z`, missing UVs to `0`.
    pub fn new(
        gl: &glow::Context,
        positions: &[[f32; 3]],
        normals: &[[f32; 3]],
        uvs: &[[f32; 2]],
        indices: &[u32],
    ) -> Self {
        let mut verts: Vec<f32> = Vec::with_capacity(positions.len() * 8);
        for (i, p) in positions.iter().enumerate() {
            let n = normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]);
            let uv = uvs.get(i).copied().unwrap_or([0.0, 0.0]);
            verts.extend_from_slice(&[p[0], p[1], p[2], n[0], n[1], n[2], uv[0], uv[1]]);
        }
        Self {
            vbo: GlBuffer::from_slice(gl, glow::ARRAY_BUFFER, &verts, glow::STATIC_DRAW),
            indices: GlBuffer::from_slice(
                gl,
                glow::ELEMENT_ARRAY_BUFFER,
                indices,
                glow::STATIC_DRAW,
            ),
            index_count: indices.len(),
        }
    }
}

// ---------------------------------------------------------------------------
// Quad buffers — shared by all drawables
// ---------------------------------------------------------------------------

pub struct QuadBuffers {
    pub verts: GlBuffer,
    pub uv: GlBuffer,
    pub indices: GlBuffer,
    pub index_count: usize,
}

pub(crate) fn build_quad(gl: &glow::Context) -> QuadBuffers {
    let verts: [f32; 12] = [
        0.0, 1.0, 0.0, // v0
        1.0, 1.0, 0.0, // v1
        0.0, 0.0, 0.0, // v2
        1.0, 0.0, 0.0, // v3
    ];
    let uvs: [f32; 8] = [
        0.0, 1.0, // uv0
        1.0, 1.0, // uv1
        0.0, 0.0, // uv2
        1.0, 0.0, // uv3
    ];
    let idx: [u16; 6] = [0, 1, 2, 1, 2, 3];

    QuadBuffers {
        verts: GlBuffer::from_slice(gl, glow::ARRAY_BUFFER, &verts, glow::STATIC_DRAW),
        uv: GlBuffer::from_slice(gl, glow::ARRAY_BUFFER, &uvs, glow::STATIC_DRAW),
        indices: GlBuffer::from_slice(gl, glow::ELEMENT_ARRAY_BUFFER, &idx, glow::STATIC_DRAW),
        index_count: idx.len(),
    }
}

// ---------------------------------------------------------------------------
// Vertex attrib setup helpers
// ---------------------------------------------------------------------------

pub(crate) fn vertex_attrib_ptr_f32(
    gl: &glow::Context,
    buffer: &GlBuffer,
    location: u32,
    components: i32,
    stride_bytes: i32,
    offset_bytes: i32,
) {
    buffer.bind(gl);
    unsafe {
        gl.vertex_attrib_pointer_f32(
            location,
            components,
            glow::FLOAT,
            false,
            stride_bytes,
            offset_bytes,
        );
        gl.enable_vertex_attrib_array(location);
    }
}

// ---------------------------------------------------------------------------
// Vertex buffer builder for dynamic geometry (SDF text glyph quads)
// ---------------------------------------------------------------------------

pub struct DynamicVb {
    buffer: glow::Buffer,
    capacity_bytes: usize,
    len_bytes: usize,
}

impl DynamicVb {
    pub fn new(gl: &glow::Context, capacity_bytes: usize) -> Self {
        let buffer = unsafe { gl.create_buffer() }.expect("create buffer");
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
            gl.buffer_data_size(glow::ARRAY_BUFFER, capacity_bytes as i32, glow::DYNAMIC_DRAW);
        }
        Self { buffer, capacity_bytes, len_bytes: 0 }
    }

    pub fn upload<T: bytemuck::Pod>(&mut self, gl: &glow::Context, data: &[T]) {
        let bytes: &[u8] = bytemuck::cast_slice(data);
        self.len_bytes = bytes.len();
        if self.len_bytes > self.capacity_bytes {
            self.capacity_bytes = self.len_bytes;
            unsafe {
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer));
                gl.buffer_data_size(
                    glow::ARRAY_BUFFER,
                    self.capacity_bytes as i32,
                    glow::DYNAMIC_DRAW,
                );
            }
        }
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer));
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes);
        }
    }

    pub fn bind(&self, gl: &glow::Context) {
        unsafe { gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer)) }
    }
}
