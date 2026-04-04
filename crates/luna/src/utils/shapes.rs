use bevy::render::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::render::render_asset::RenderAssetUsages;
use std::f32::consts::FRAC_PI_2;

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
