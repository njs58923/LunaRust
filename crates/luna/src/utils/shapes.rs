use bevy::render::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::render::render_asset::RenderAssetUsages;
use std::f32::consts::{FRAC_PI_2, PI};

pub fn create_plane() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-0.5, -0.5, 0.0],
            [0.5, -0.5, 0.0],
            [0.5, 0.5, 0.0],
            [-0.5, 0.5, 0.0],
        ],
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vec![
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
    )
    .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]))
}

pub fn create_cube() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            // Cara frontal
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [0.5, 0.5, 0.5],
            [-0.5, 0.5, 0.5],
            // Cara trasera
            [0.5, -0.5, -0.5],
            [-0.5, -0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [0.5, 0.5, -0.5],
            // Cara izquierda
            [-0.5, -0.5, -0.5],
            [-0.5, -0.5, 0.5],
            [-0.5, 0.5, 0.5],
            [-0.5, 0.5, -0.5],
            // Cara derecha
            [0.5, -0.5, 0.5],
            [0.5, -0.5, -0.5],
            [0.5, 0.5, -0.5],
            [0.5, 0.5, 0.5],
            // Cara superior
            [-0.5, 0.5, 0.5],
            [0.5, 0.5, 0.5],
            [0.5, 0.5, -0.5],
            [-0.5, 0.5, -0.5],
            // Cara inferior
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [0.5, -0.5, 0.5],
            [-0.5, -0.5, 0.5],
        ],
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![
            // Cada cara con UVs de esquina inferior izquierda a superior derecha
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
            // Trasera
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
            // Izquierda
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
            // Derecha
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
            // Superior
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
            // Inferior
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
        ],
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vec![
            // Normal para la cara frontal
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            // Normal para la cara trasera
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            // Normal para la cara izquierda
            [-1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            // Normal para la cara derecha
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            // Normal para la cara superior
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            // Normal para la cara inferior
            [0.0, -1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, -1.0, 0.0],
        ],
    )
    .with_inserted_indices(Indices::U32(vec![
        // Cara frontal
        0, 1, 2, 0, 2, 3, // Cara trasera
        4, 5, 6, 4, 6, 7, // Cara izquierda
        8, 9, 10, 8, 10, 11, // Cara derecha
        12, 13, 14, 12, 14, 15, // Cara superior
        16, 17, 18, 16, 18, 19, // Cara inferior
        20, 21, 22, 20, 22, 23,
    ]))
}

/// Creates a unit cube (-0.5..0.5) with rounded edges and corners.
/// `radius` is the rounding radius (clamped to 0..0.5).
/// `segments` controls smoothness of the curves (typically 4-8).
pub fn create_rounded_cube(radius: f32, segments: u32) -> Mesh {
    let r = radius.clamp(0.0, 0.499);
    let seg = segments.max(1);

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Half-extent minus radius = where the flat part ends
    let h = 0.5 - r;

    // Helper: check if the first triangle of a strip/patch faces outward by comparing
    // its geometric normal (cross product) against the stored vertex normal.
    // Returns true if winding needs to be flipped.
    let needs_flip =
        |positions: &[[f32; 3]], normals: &[[f32; 3]], a: u32, b: u32, c: u32| -> bool {
            let pa = positions[a as usize];
            let pb = positions[b as usize];
            let pc = positions[c as usize];
            let e1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
            let e2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
            let cx = e1[1] * e2[2] - e1[2] * e2[1];
            let cy = e1[2] * e2[0] - e1[0] * e2[2];
            let cz = e1[0] * e2[1] - e1[1] * e2[0];
            let vn = normals[a as usize];
            let dot = cx * vn[0] + cy * vn[1] + cz * vn[2];
            dot < 0.0
        };

    // --- 6 FACES (flat quads, inset by radius) ---
    struct FaceInfo {
        normal: [f32; 3],
        up: [f32; 3],
        right: [f32; 3],
        center: [f32; 3],
    }

    let faces = [
        FaceInfo {
            normal: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
            right: [1.0, 0.0, 0.0],
            center: [0.0, 0.0, 0.5],
        },
        FaceInfo {
            normal: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            right: [-1.0, 0.0, 0.0],
            center: [0.0, 0.0, -0.5],
        },
        FaceInfo {
            normal: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            right: [0.0, 0.0, -1.0],
            center: [0.5, 0.0, 0.0],
        },
        FaceInfo {
            normal: [-1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            right: [0.0, 0.0, 1.0],
            center: [-0.5, 0.0, 0.0],
        },
        FaceInfo {
            normal: [0.0, 1.0, 0.0],
            up: [0.0, 0.0, -1.0],
            right: [1.0, 0.0, 0.0],
            center: [0.0, 0.5, 0.0],
        },
        FaceInfo {
            normal: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            right: [1.0, 0.0, 0.0],
            center: [0.0, -0.5, 0.0],
        },
    ];

    for face in &faces {
        let base = positions.len() as u32;
        let n = face.normal;
        for &v in &[-1.0_f32, 1.0] {
            for &u in &[-1.0_f32, 1.0] {
                let px = face.center[0] + face.right[0] * u * h + face.up[0] * v * h;
                let py = face.center[1] + face.right[1] * u * h + face.up[1] * v * h;
                let pz = face.center[2] + face.right[2] * u * h + face.up[2] * v * h;
                positions.push([px, py, pz]);
                normals.push(n);
                uvs.push([(u + 1.0) * 0.5, (1.0 - v) * 0.5]);
            }
        }
        // 0=(-1,-1), 1=(1,-1), 2=(-1,1), 3=(1,1)
        let flip = needs_flip(&positions, &normals, base, base + 1, base + 3);
        if flip {
            indices.extend_from_slice(&[base, base + 3, base + 1, base, base + 2, base + 3]);
        } else {
            indices.extend_from_slice(&[base, base + 1, base + 3, base, base + 3, base + 2]);
        }
    }

    // --- 12 EDGES (cylindrical strips) ---
    struct EdgeInfo {
        p0: [f32; 3],
        p1: [f32; 3],
        n0: [f32; 3],
        n1: [f32; 3],
    }

    let edges = [
        // Front face edges (z = +h)
        EdgeInfo {
            p0: [-h, -h, h],
            p1: [h, -h, h],
            n0: [0.0, 0.0, 1.0],
            n1: [0.0, -1.0, 0.0],
        },
        EdgeInfo {
            p0: [-h, h, h],
            p1: [h, h, h],
            n0: [0.0, 1.0, 0.0],
            n1: [0.0, 0.0, 1.0],
        },
        EdgeInfo {
            p0: [-h, -h, h],
            p1: [-h, h, h],
            n0: [0.0, 0.0, 1.0],
            n1: [-1.0, 0.0, 0.0],
        },
        EdgeInfo {
            p0: [h, -h, h],
            p1: [h, h, h],
            n0: [1.0, 0.0, 0.0],
            n1: [0.0, 0.0, 1.0],
        },
        // Back face edges (z = -h)
        EdgeInfo {
            p0: [h, -h, -h],
            p1: [-h, -h, -h],
            n0: [0.0, 0.0, -1.0],
            n1: [0.0, -1.0, 0.0],
        },
        EdgeInfo {
            p0: [h, h, -h],
            p1: [-h, h, -h],
            n0: [0.0, 1.0, 0.0],
            n1: [0.0, 0.0, -1.0],
        },
        EdgeInfo {
            p0: [h, -h, -h],
            p1: [h, h, -h],
            n0: [0.0, 0.0, -1.0],
            n1: [1.0, 0.0, 0.0],
        },
        EdgeInfo {
            p0: [-h, -h, -h],
            p1: [-h, h, -h],
            n0: [-1.0, 0.0, 0.0],
            n1: [0.0, 0.0, -1.0],
        },
        // Connecting edges (along z)
        EdgeInfo {
            p0: [-h, -h, h],
            p1: [-h, -h, -h],
            n0: [0.0, -1.0, 0.0],
            n1: [-1.0, 0.0, 0.0],
        },
        EdgeInfo {
            p0: [h, -h, h],
            p1: [h, -h, -h],
            n0: [0.0, -1.0, 0.0],
            n1: [1.0, 0.0, 0.0],
        },
        EdgeInfo {
            p0: [-h, h, h],
            p1: [-h, h, -h],
            n0: [-1.0, 0.0, 0.0],
            n1: [0.0, 1.0, 0.0],
        },
        EdgeInfo {
            p0: [h, h, h],
            p1: [h, h, -h],
            n0: [1.0, 0.0, 0.0],
            n1: [0.0, 1.0, 0.0],
        },
    ];

    for edge in &edges {
        let base = positions.len() as u32;
        for i in 0..=seg {
            let t = i as f32 / seg as f32;
            let angle = t * FRAC_PI_2;
            let cos_a = angle.cos();
            let sin_a = angle.sin();

            let nx = edge.n0[0] * cos_a + edge.n1[0] * sin_a;
            let ny = edge.n0[1] * cos_a + edge.n1[1] * sin_a;
            let nz = edge.n0[2] * cos_a + edge.n1[2] * sin_a;

            for p in &[edge.p0, edge.p1] {
                positions.push([p[0] + nx * r, p[1] + ny * r, p[2] + nz * r]);
                normals.push([nx, ny, nz]);
                uvs.push([t, 0.0]);
            }
        }

        // Check winding on the first quad: triangle (a, c, d) where a=p0_0, c=p0_1, d=p1_1
        let flip = needs_flip(&positions, &normals, base, base + 2, base + 3);

        for i in 0..seg {
            let a = base + i * 2;
            let b = a + 1;
            let c = a + 2;
            let d = a + 3;
            if flip {
                indices.extend_from_slice(&[a, d, c, a, b, d]);
            } else {
                indices.extend_from_slice(&[a, c, d, a, d, b]);
            }
        }
    }

    // --- 8 CORNERS (spherical patches) ---
    struct CornerInfo {
        center: [f32; 3],
        nx: [f32; 3],
        ny: [f32; 3],
        nz: [f32; 3],
    }

    let corners = [
        CornerInfo {
            center: [h, h, h],
            nx: [1.0, 0.0, 0.0],
            ny: [0.0, 1.0, 0.0],
            nz: [0.0, 0.0, 1.0],
        },
        CornerInfo {
            center: [-h, h, h],
            nx: [-1.0, 0.0, 0.0],
            ny: [0.0, 1.0, 0.0],
            nz: [0.0, 0.0, 1.0],
        },
        CornerInfo {
            center: [h, -h, h],
            nx: [1.0, 0.0, 0.0],
            ny: [0.0, -1.0, 0.0],
            nz: [0.0, 0.0, 1.0],
        },
        CornerInfo {
            center: [-h, -h, h],
            nx: [-1.0, 0.0, 0.0],
            ny: [0.0, -1.0, 0.0],
            nz: [0.0, 0.0, 1.0],
        },
        CornerInfo {
            center: [h, h, -h],
            nx: [1.0, 0.0, 0.0],
            ny: [0.0, 1.0, 0.0],
            nz: [0.0, 0.0, -1.0],
        },
        CornerInfo {
            center: [-h, h, -h],
            nx: [-1.0, 0.0, 0.0],
            ny: [0.0, 1.0, 0.0],
            nz: [0.0, 0.0, -1.0],
        },
        CornerInfo {
            center: [h, -h, -h],
            nx: [1.0, 0.0, 0.0],
            ny: [0.0, -1.0, 0.0],
            nz: [0.0, 0.0, -1.0],
        },
        CornerInfo {
            center: [-h, -h, -h],
            nx: [-1.0, 0.0, 0.0],
            ny: [0.0, -1.0, 0.0],
            nz: [0.0, 0.0, -1.0],
        },
    ];

    for corner in &corners {
        let base = positions.len() as u32;
        let stride = seg + 1;
        for j in 0..=seg {
            let v = j as f32 / seg as f32;
            let theta = v * FRAC_PI_2;
            for i in 0..=seg {
                let u = i as f32 / seg as f32;
                let phi = u * FRAC_PI_2;

                let cos_theta = theta.cos();
                let sin_theta = theta.sin();
                let cos_phi = phi.cos();
                let sin_phi = phi.sin();

                let dx = corner.nx[0] * sin_phi * cos_theta
                    + corner.ny[0] * sin_theta
                    + corner.nz[0] * cos_phi * cos_theta;
                let dy = corner.nx[1] * sin_phi * cos_theta
                    + corner.ny[1] * sin_theta
                    + corner.nz[1] * cos_phi * cos_theta;
                let dz = corner.nx[2] * sin_phi * cos_theta
                    + corner.ny[2] * sin_theta
                    + corner.nz[2] * cos_phi * cos_theta;

                let len = (dx * dx + dy * dy + dz * dz).sqrt();
                let nx = dx / len;
                let ny = dy / len;
                let nz = dz / len;

                positions.push([
                    corner.center[0] + nx * r,
                    corner.center[1] + ny * r,
                    corner.center[2] + nz * r,
                ]);
                normals.push([nx, ny, nz]);
                uvs.push([u, v]);
            }
        }

        // Check winding on the first triangle: (0,0) -> (1,0) -> (1,1)
        let flip = needs_flip(&positions, &normals, base, base + stride, base + stride + 1);

        for j in 0..seg {
            for i in 0..seg {
                let a = base + j * stride + i;
                let b = a + 1;
                let c = a + stride;
                let d = c + 1;
                if flip {
                    indices.extend_from_slice(&[a, d, c, a, b, d]);
                } else {
                    indices.extend_from_slice(&[a, c, d, a, d, b]);
                }
            }
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// Rounded cube with per-axis radii (rx, ry, rz) in local space.
/// Ensures circular front-face corners in world space when called with
/// rx = r_world/sx, ry = r_world/sy so that rx*sx == ry*sy.
pub fn create_rounded_cube_aniso(rx: f32, ry: f32, rz: f32, segments: u32) -> Mesh {
    let rx = rx.clamp(0.0, 0.499);
    let ry = ry.clamp(0.0, 0.499);
    let rz = rz.clamp(0.0, 0.499);
    let hx = 0.5 - rx;
    let hy = 0.5 - ry;
    let hz = 0.5 - rz;
    let seg = segments.max(1);

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let needs_flip =
        |positions: &[[f32; 3]], normals: &[[f32; 3]], a: u32, b: u32, c: u32| -> bool {
            let pa = positions[a as usize];
            let pb = positions[b as usize];
            let pc = positions[c as usize];
            let e1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
            let e2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
            let cx = e1[1] * e2[2] - e1[2] * e2[1];
            let cy = e1[2] * e2[0] - e1[0] * e2[2];
            let cz = e1[0] * e2[1] - e1[1] * e2[0];
            let vn = normals[a as usize];
            cx * vn[0] + cy * vn[1] + cz * vn[2] < 0.0
        };

    // --- 6 FACES ---
    struct FaceInfo {
        normal: [f32; 3],
        right: [f32; 3],
        up: [f32; 3],
        hr: f32,
        hu: f32,
        center: [f32; 3],
    }
    let faces = [
        FaceInfo { normal: [0., 0., 1.],  right: [1., 0., 0.],  up: [0., 1., 0.],  hr: hx, hu: hy, center: [0., 0., 0.5]  },
        FaceInfo { normal: [0., 0., -1.], right: [-1., 0., 0.], up: [0., 1., 0.],  hr: hx, hu: hy, center: [0., 0., -0.5] },
        FaceInfo { normal: [1., 0., 0.],  right: [0., 0., -1.], up: [0., 1., 0.],  hr: hz, hu: hy, center: [0.5, 0., 0.]  },
        FaceInfo { normal: [-1., 0., 0.], right: [0., 0., 1.],  up: [0., 1., 0.],  hr: hz, hu: hy, center: [-0.5, 0., 0.] },
        FaceInfo { normal: [0., 1., 0.],  right: [1., 0., 0.],  up: [0., 0., -1.], hr: hx, hu: hz, center: [0., 0.5, 0.]  },
        FaceInfo { normal: [0., -1., 0.], right: [1., 0., 0.],  up: [0., 0., 1.],  hr: hx, hu: hz, center: [0., -0.5, 0.] },
    ];

    for face in &faces {
        let base = positions.len() as u32;
        let n = face.normal;
        for &v in &[-1.0_f32, 1.0] {
            for &u in &[-1.0_f32, 1.0] {
                let px = face.center[0] + face.right[0] * u * face.hr + face.up[0] * v * face.hu;
                let py = face.center[1] + face.right[1] * u * face.hr + face.up[1] * v * face.hu;
                let pz = face.center[2] + face.right[2] * u * face.hr + face.up[2] * v * face.hu;
                positions.push([px, py, pz]);
                normals.push(n);
                uvs.push([(u + 1.) * 0.5, (1. - v) * 0.5]);
            }
        }
        let flip = needs_flip(&positions, &normals, base, base + 1, base + 3);
        if flip {
            indices.extend_from_slice(&[base, base + 3, base + 1, base, base + 2, base + 3]);
        } else {
            indices.extend_from_slice(&[base, base + 1, base + 3, base, base + 3, base + 2]);
        }
    }

    // --- 12 EDGES (elliptical arcs) ---
    // r0/r1: arc radii along the n0/n1 face-normal directions.
    // Normal = gradient of ellipse: normalize(n0*cos/r0 + n1*sin/r1).
    struct EdgeInfo {
        p0: [f32; 3],
        p1: [f32; 3],
        n0: [f32; 3],
        n1: [f32; 3],
        r0: f32,
        r1: f32,
    }
    let edges = [
        // Z-parallel (XY corners), arc in XY using rx, ry
        EdgeInfo { p0: [hx, hy, -hz],   p1: [hx, hy, hz],   n0: [1., 0., 0.],  n1: [0., 1., 0.],  r0: rx, r1: ry },
        EdgeInfo { p0: [-hx, hy, -hz],  p1: [-hx, hy, hz],  n0: [0., 1., 0.],  n1: [-1., 0., 0.], r0: ry, r1: rx },
        EdgeInfo { p0: [hx, -hy, -hz],  p1: [hx, -hy, hz],  n0: [0., -1., 0.], n1: [1., 0., 0.],  r0: ry, r1: rx },
        EdgeInfo { p0: [-hx, -hy, -hz], p1: [-hx, -hy, hz], n0: [-1., 0., 0.], n1: [0., -1., 0.], r0: rx, r1: ry },
        // Y-parallel (XZ corners), arc in XZ using rx, rz
        EdgeInfo { p0: [hx, -hy, hz],   p1: [hx, hy, hz],   n0: [1., 0., 0.],  n1: [0., 0., 1.],  r0: rx, r1: rz },
        EdgeInfo { p0: [hx, -hy, -hz],  p1: [hx, hy, -hz],  n0: [0., 0., -1.], n1: [1., 0., 0.],  r0: rz, r1: rx },
        EdgeInfo { p0: [-hx, -hy, hz],  p1: [-hx, hy, hz],  n0: [0., 0., 1.],  n1: [-1., 0., 0.], r0: rz, r1: rx },
        EdgeInfo { p0: [-hx, -hy, -hz], p1: [-hx, hy, -hz], n0: [-1., 0., 0.], n1: [0., 0., -1.], r0: rx, r1: rz },
        // X-parallel (YZ corners), arc in YZ using ry, rz
        EdgeInfo { p0: [-hx, hy, hz],   p1: [hx, hy, hz],   n0: [0., 1., 0.],  n1: [0., 0., 1.],  r0: ry, r1: rz },
        EdgeInfo { p0: [-hx, hy, -hz],  p1: [hx, hy, -hz],  n0: [0., 0., -1.], n1: [0., 1., 0.],  r0: rz, r1: ry },
        EdgeInfo { p0: [-hx, -hy, hz],  p1: [hx, -hy, hz],  n0: [0., 0., 1.],  n1: [0., -1., 0.], r0: rz, r1: ry },
        EdgeInfo { p0: [-hx, -hy, -hz], p1: [hx, -hy, -hz], n0: [0., -1., 0.], n1: [0., 0., -1.], r0: ry, r1: rz },
    ];

    for edge in &edges {
        let base = positions.len() as u32;
        for i in 0..=seg {
            let t = i as f32 / seg as f32;
            let angle = t * FRAC_PI_2;
            let cos_a = angle.cos();
            let sin_a = angle.sin();

            let dx = edge.n0[0] * cos_a * edge.r0 + edge.n1[0] * sin_a * edge.r1;
            let dy = edge.n0[1] * cos_a * edge.r0 + edge.n1[1] * sin_a * edge.r1;
            let dz = edge.n0[2] * cos_a * edge.r0 + edge.n1[2] * sin_a * edge.r1;

            let nx_raw = edge.n0[0] * cos_a / edge.r0 + edge.n1[0] * sin_a / edge.r1;
            let ny_raw = edge.n0[1] * cos_a / edge.r0 + edge.n1[1] * sin_a / edge.r1;
            let nz_raw = edge.n0[2] * cos_a / edge.r0 + edge.n1[2] * sin_a / edge.r1;
            let nlen = (nx_raw * nx_raw + ny_raw * ny_raw + nz_raw * nz_raw)
                .sqrt()
                .max(0.0001);

            for p in &[edge.p0, edge.p1] {
                positions.push([p[0] + dx, p[1] + dy, p[2] + dz]);
                normals.push([nx_raw / nlen, ny_raw / nlen, nz_raw / nlen]);
                uvs.push([t, 0.0]);
            }
        }

        let flip = needs_flip(&positions, &normals, base, base + 2, base + 3);
        for i in 0..seg {
            let a = base + i * 2;
            let b = a + 1;
            let c = a + 2;
            let d = a + 3;
            if flip {
                indices.extend_from_slice(&[a, d, c, a, b, d]);
            } else {
                indices.extend_from_slice(&[a, c, d, a, d, b]);
            }
        }
    }

    // --- 8 CORNERS (ellipsoidal patches) ---
    struct CornerInfo {
        center: [f32; 3],
        nx_dir: [f32; 3],
        ny_dir: [f32; 3],
        nz_dir: [f32; 3],
    }
    let corners = [
        CornerInfo { center: [ hx,  hy,  hz], nx_dir: [1., 0., 0.],  ny_dir: [0., 1., 0.],  nz_dir: [0., 0., 1.]  },
        CornerInfo { center: [-hx,  hy,  hz], nx_dir: [-1., 0., 0.], ny_dir: [0., 1., 0.],  nz_dir: [0., 0., 1.]  },
        CornerInfo { center: [ hx, -hy,  hz], nx_dir: [1., 0., 0.],  ny_dir: [0., -1., 0.], nz_dir: [0., 0., 1.]  },
        CornerInfo { center: [-hx, -hy,  hz], nx_dir: [-1., 0., 0.], ny_dir: [0., -1., 0.], nz_dir: [0., 0., 1.]  },
        CornerInfo { center: [ hx,  hy, -hz], nx_dir: [1., 0., 0.],  ny_dir: [0., 1., 0.],  nz_dir: [0., 0., -1.] },
        CornerInfo { center: [-hx,  hy, -hz], nx_dir: [-1., 0., 0.], ny_dir: [0., 1., 0.],  nz_dir: [0., 0., -1.] },
        CornerInfo { center: [ hx, -hy, -hz], nx_dir: [1., 0., 0.],  ny_dir: [0., -1., 0.], nz_dir: [0., 0., -1.] },
        CornerInfo { center: [-hx, -hy, -hz], nx_dir: [-1., 0., 0.], ny_dir: [0., -1., 0.], nz_dir: [0., 0., -1.] },
    ];

    for corner in &corners {
        let base = positions.len() as u32;
        let stride = seg + 1;
        for j in 0..=seg {
            let v = j as f32 / seg as f32;
            let theta = v * FRAC_PI_2;
            for i in 0..=seg {
                let u = i as f32 / seg as f32;
                let phi = u * FRAC_PI_2;
                let cos_theta = theta.cos();
                let sin_theta = theta.sin();
                let cos_phi = phi.cos();
                let sin_phi = phi.sin();

                let ux = sin_phi * cos_theta;
                let uy = sin_theta;
                let uz = cos_phi * cos_theta;

                let px = corner.center[0]
                    + corner.nx_dir[0] * ux * rx
                    + corner.ny_dir[0] * uy * ry
                    + corner.nz_dir[0] * uz * rz;
                let py = corner.center[1]
                    + corner.nx_dir[1] * ux * rx
                    + corner.ny_dir[1] * uy * ry
                    + corner.nz_dir[1] * uz * rz;
                let pz = corner.center[2]
                    + corner.nx_dir[2] * ux * rx
                    + corner.ny_dir[2] * uy * ry
                    + corner.nz_dir[2] * uz * rz;

                // Ellipsoid normal = gradient of (dx/rx)^2+(dy/ry)^2+(dz/rz)^2=1
                let gnx = corner.nx_dir[0] * ux / rx
                    + corner.ny_dir[0] * uy / ry
                    + corner.nz_dir[0] * uz / rz;
                let gny = corner.nx_dir[1] * ux / rx
                    + corner.ny_dir[1] * uy / ry
                    + corner.nz_dir[1] * uz / rz;
                let gnz = corner.nx_dir[2] * ux / rx
                    + corner.ny_dir[2] * uy / ry
                    + corner.nz_dir[2] * uz / rz;
                let glen = (gnx * gnx + gny * gny + gnz * gnz).sqrt().max(0.0001);

                positions.push([px, py, pz]);
                normals.push([gnx / glen, gny / glen, gnz / glen]);
                uvs.push([u, v]);
            }
        }

        let flip = needs_flip(&positions, &normals, base, base + stride, base + stride + 1);
        for j in 0..seg {
            for i in 0..seg {
                let a = base + j * stride + i;
                let b = a + 1;
                let c = a + stride;
                let d = c + 1;
                if flip {
                    indices.extend_from_slice(&[a, d, c, a, b, d]);
                } else {
                    indices.extend_from_slice(&[a, c, d, a, d, b]);
                }
            }
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// Flat rounded-corner plane (Z=0, +Z normals, single-sided).
/// `radius` is clamped to [0, 0.499]. `segments` controls arc smoothness.
pub fn create_rounded_plane(radius: f32, segments: u32) -> Mesh {
    let r = radius.clamp(0.0, 0.499);
    let seg = segments.max(1);
    let h = 0.5 - r; // half-extent minus radius

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Center vertex (index 0)
    positions.push([0.0, 0.0, 0.0]);
    normals.push([0.0, 0.0, 1.0]);
    uvs.push([0.5, 0.5]);

    // Build CCW perimeter:
    // bottom edge (left to right), bottom-right arc, right edge (bottom to top),
    // top-right arc, top edge (right to left), top-left arc,
    // left edge (top to bottom), bottom-left arc.
    //
    // Corner arcs: (seg+1) vertices each, angle goes from start to end inclusive.

    // bottom edge: (-h, -h) to (h, -h)
    for i in 0..=0 {
        let _ = i;
        // just the two endpoints, but straight edges are implicit between arc endpoints
    }

    // We'll push perimeter vertices in order. Straight edges contribute only their
    // endpoints (corners are shared with adjacent arcs).
    //
    // Layout: 4 arcs * (seg+1) vertices each = 4*(seg+1) perimeter vertices.
    // Between consecutive arc endpoint and next arc startpoint we draw straight
    // triangle fans that include any intermediate straight vertices.
    // For simplicity we only emit straight-edge midpoints when needed — here since
    // we want all mesh triangles to be fans from center, straight edge midpoints
    // are optional. We add them to reduce sliver triangles on long edges.
    //
    // Final perimeter: for each corner arc emit (seg+1) verts, straight edge
    // midpoints between corners share the arc endpoint verts.

    // Arc corners: center positions, start/end angles (CCW)
    // bottom-right: center=(h, -h), angles -PI/2 → 0
    // top-right:    center=(h,  h), angles 0 → PI/2
    // top-left:     center=(-h, h), angles PI/2 → PI
    // bottom-left:  center=(-h,-h), angles PI → 3*PI/2
    struct Corner {
        cx: f32,
        cy: f32,
        a_start: f32,
        a_end: f32,
    }
    let corners = [
        Corner { cx:  h, cy: -h, a_start: -FRAC_PI_2,        a_end: 0.0             },
        Corner { cx:  h, cy:  h, a_start: 0.0,               a_end: FRAC_PI_2       },
        Corner { cx: -h, cy:  h, a_start: FRAC_PI_2,         a_end: PI              },
        Corner { cx: -h, cy: -h, a_start: PI,                a_end: 3.0 * FRAC_PI_2 },
    ];

    // Emit all perimeter vertices.
    // Each arc emits seg+1 verts. The last vert of arc[i] == first vert of arc[i+1]
    // so we skip the last vert of each arc and close the loop at triangle emit time.
    let perimeter_start = positions.len() as u32; // = 1
    let verts_per_arc = seg + 1;

    for corner in &corners {
        for i in 0..verts_per_arc {
            // skip the very last vertex of each arc (it equals first of next arc)
            if i == seg { continue; }
            let t = i as f32 / seg as f32;
            let angle = corner.a_start + t * (corner.a_end - corner.a_start);
            let x = corner.cx + r * angle.cos();
            let y = corner.cy + r * angle.sin();
            positions.push([x, y, 0.0]);
            normals.push([0.0, 0.0, 1.0]);
            uvs.push([x + 0.5, 0.5 - y]);
        }
    }

    // Number of perimeter vertices (each arc contributes `seg` verts, 4 arcs total)
    let perim_count = (seg * 4) as u32;

    // Fan triangles: center(0) + consecutive perimeter pairs
    for i in 0..perim_count {
        let a = perimeter_start + i;
        let b = perimeter_start + (i + 1) % perim_count;
        indices.extend_from_slice(&[0, a, b]);
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}
