
use bevy::render::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::render::render_asset::RenderAssetUsages;

pub fn create_plane() -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [-0.5, -0.5, 0.0],
                [ 0.5, -0.5, 0.0],
                [ 0.5,  0.5, 0.0],
                [-0.5,  0.5, 0.0],
            ],
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_UV_0,
            vec![
                [0.0, 1.0],
                [1.0, 1.0],
                [1.0, 0.0],
                [0.0, 0.0],
            ],
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
        .with_inserted_indices(Indices::U32(vec![
            0, 1, 2,
            0, 2, 3,
        ]))
}

pub fn create_cube() -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                // Cara frontal
                [-0.5, -0.5,  0.5],
                [ 0.5, -0.5,  0.5],
                [ 0.5,  0.5,  0.5],
                [-0.5,  0.5,  0.5],
                // Cara trasera
                [ 0.5, -0.5, -0.5],
                [-0.5, -0.5, -0.5],
                [-0.5,  0.5, -0.5],
                [ 0.5,  0.5, -0.5],
                // Cara izquierda
                [-0.5, -0.5, -0.5],
                [-0.5, -0.5,  0.5],
                [-0.5,  0.5,  0.5],
                [-0.5,  0.5, -0.5],
                // Cara derecha
                [ 0.5, -0.5,  0.5],
                [ 0.5, -0.5, -0.5],
                [ 0.5,  0.5, -0.5],
                [ 0.5,  0.5,  0.5],
                // Cara superior
                [-0.5,  0.5,  0.5],
                [ 0.5,  0.5,  0.5],
                [ 0.5,  0.5, -0.5],
                [-0.5,  0.5, -0.5],
                // Cara inferior
                [-0.5, -0.5, -0.5],
                [ 0.5, -0.5, -0.5],
                [ 0.5, -0.5,  0.5],
                [-0.5, -0.5,  0.5],
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
            0, 1, 2, 0, 2, 3,
            // Cara trasera
            4, 5, 6, 4, 6, 7,
            // Cara izquierda
            8, 9, 10, 8, 10, 11,
            // Cara derecha
            12, 13, 14, 12, 14, 15,
            // Cara superior
            16, 17, 18, 16, 18, 19,
            // Cara inferior
            20, 21, 22, 20, 22, 23,
        ]))
}
