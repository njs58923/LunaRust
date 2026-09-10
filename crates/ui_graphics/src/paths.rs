//! Bounded vector paths in layout XY coordinates, tessellated only when requested.
use lyon_tessellation::{
    path::{iterator::PathIterator, math::point, Event, Path},
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, LineCap, LineJoin,
    StrokeOptions, StrokeTessellator, StrokeVertex, VertexBuffers,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Command {
    MoveTo {
        x: f32,
        y: f32,
    },
    LineTo {
        x: f32,
        y: f32,
    },
    QuadraticTo {
        cx: f32,
        cy: f32,
        x: f32,
        y: f32,
    },
    BezierTo {
        c1x: f32,
        c1y: f32,
        c2x: f32,
        c2y: f32,
        x: f32,
        y: f32,
    },
    Close,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub commands: Vec<Command>,
    pub tolerance: f32,
    pub stroke_width: f32,
    pub non_zero: bool,
}
#[derive(Serialize)]
pub struct Geometry {
    pub positions: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub contours: Vec<Vec<[f32; 2]>>,
    pub closed: Vec<bool>,
}
pub fn tessellate(request: Request) -> Result<Geometry, String> {
    if request.commands.len() > 128 || request.commands.is_empty() {
        return Err("Path requires 1..128 commands".into());
    }
    if !request.tolerance.is_finite()
        || !(0.00001..=0.1).contains(&request.tolerance)
        || !request.stroke_width.is_finite()
        || !(0.0..=10.0).contains(&request.stroke_width)
    {
        return Err("Invalid path tolerance or stroke width".into());
    }
    fn valid(values: &[f32]) -> Result<(), String> {
        if values.iter().any(|v| !v.is_finite() || v.abs() > 1000.0) {
            Err("Path coordinates must be finite and within 1000 units".into())
        } else {
            Ok(())
        }
    }
    let mut builder = Path::builder();
    let mut open = false;
    for command in request.commands {
        match command {
            Command::MoveTo { x, y } => {
                valid(&[x, y])?;
                if open {
                    builder.end(false);
                }
                builder.begin(point(x, y));
                open = true;
            }
            Command::Close => {
                if !open {
                    return Err("close without moveTo".into());
                }
                builder.end(true);
                open = false;
            }
            command => {
                if !open {
                    return Err("Path segment requires moveTo".into());
                }
                match command {
                    Command::LineTo { x, y } => {
                        valid(&[x, y])?;
                        builder.line_to(point(x, y));
                    }
                    Command::QuadraticTo { cx, cy, x, y } => {
                        valid(&[cx, cy, x, y])?;
                        builder.quadratic_bezier_to(point(cx, cy), point(x, y));
                    }
                    Command::BezierTo {
                        c1x,
                        c1y,
                        c2x,
                        c2y,
                        x,
                        y,
                    } => {
                        valid(&[c1x, c1y, c2x, c2y, x, y])?;
                        builder.cubic_bezier_to(point(c1x, c1y), point(c2x, c2y), point(x, y));
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
    if open {
        builder.end(false);
    }
    let path = builder.build();
    let mut flat = Path::builder();
    let mut contours = Vec::new();
    let mut closed = Vec::new();
    let mut points = Vec::new();
    for (n, event) in path.iter().flattened(request.tolerance).enumerate() {
        if n >= 512 {
            return Err(
                "Path exceeds 512 flattened segments; increase tolerance or split path".into(),
            );
        }
        match event {
            Event::Begin { at } => {
                flat.begin(at);
                points = vec![[at.x, at.y]];
            }
            Event::Line { to, .. } => {
                flat.line_to(to);
                points.push([to.x, to.y]);
            }
            Event::End { close, .. } => {
                flat.end(close);
                if points.len() > 1 && points.first() == points.last() {
                    points.pop();
                }
                contours.push(std::mem::take(&mut points));
                closed.push(close);
            }
            _ => unreachable!(),
        }
    }
    let flat = flat.build();
    let mut output: VertexBuffers<[f32; 2], u16> = VertexBuffers::new();
    if request.stroke_width > 0.0 {
        StrokeTessellator::new()
            .tessellate_path(
                &flat,
                &StrokeOptions::default()
                    .with_line_width(request.stroke_width)
                    .with_tolerance(request.tolerance)
                    .with_line_cap(LineCap::Round)
                    .with_line_join(LineJoin::Round),
                &mut BuffersBuilder::new(&mut output, |v: StrokeVertex| {
                    [v.position().x, v.position().y]
                }),
            )
            .map_err(|e| format!("Path stroke: {e:?}"))?;
    } else {
        FillTessellator::new()
            .tessellate_path(
                &flat,
                &FillOptions::default()
                    .with_tolerance(request.tolerance)
                    .with_fill_rule(if request.non_zero {
                        FillRule::NonZero
                    } else {
                        FillRule::EvenOdd
                    }),
                &mut BuffersBuilder::new(&mut output, |v: FillVertex| {
                    [v.position().x, v.position().y]
                }),
            )
            .map_err(|e| format!("Path fill: {e:?}"))?;
    }
    if output.vertices.len() > 65536 || output.indices.len() > 196608 {
        return Err("Path tessellation exceeds mesh budget".into());
    }
    for triangle in output.indices.chunks_exact_mut(3) {
        let a = output.vertices[triangle[0] as usize];
        let b = output.vertices[triangle[1] as usize];
        let c = output.vertices[triangle[2] as usize];
        if (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]) < 0.0 {
            triangle.swap(1, 2);
        }
    }
    Ok(Geometry {
        positions: output.vertices,
        indices: output.indices.into_iter().map(u32::from).collect(),
        contours,
        closed,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request(commands: Vec<Command>, tolerance: f32) -> Request {
        Request {
            commands,
            tolerance,
            stroke_width: 0.0,
            non_zero: false,
        }
    }
    #[test]
    fn adaptive_curves_and_round_strokes() {
        let c = vec![
            Command::MoveTo { x: 0.0, y: 0.0 },
            Command::BezierTo {
                c1x: 0.0,
                c1y: 1.0,
                c2x: 1.0,
                c2y: 1.0,
                x: 1.0,
                y: 0.0,
            },
            Command::Close,
        ];
        let coarse = tessellate(request(c.clone(), 0.01)).unwrap();
        let fine = tessellate(request(c, 0.0001)).unwrap();
        assert!(fine.positions.len() > coarse.positions.len());
        let mut r = request(
            vec![
                Command::MoveTo { x: 0.0, y: 0.0 },
                Command::QuadraticTo {
                    cx: 0.5,
                    cy: 1.0,
                    x: 1.0,
                    y: 0.0,
                },
            ],
            0.001,
        );
        r.stroke_width = 0.02;
        assert!(!tessellate(r).unwrap().indices.is_empty());
    }
    #[test]
    fn invalid_paths_and_complexity_are_rejected() {
        assert!(tessellate(request(vec![Command::Close], 0.001)).is_err());
        assert!(tessellate(request(
            vec![Command::MoveTo {
                x: f32::NAN,
                y: 0.0
            }],
            0.001
        ))
        .is_err());
        assert!(tessellate(request(vec![Command::MoveTo { x: 0.0, y: 0.0 }], 0.0)).is_err());
    }
    #[test]
    fn holes_preserve_even_odd_area_and_front_winding() {
        let mut commands = Vec::new();
        for (a, b) in [(0.0, 1.0), (0.25, 0.75)] {
            commands.extend([
                Command::MoveTo { x: a, y: a },
                Command::LineTo { x: b, y: a },
                Command::LineTo { x: b, y: b },
                Command::LineTo { x: a, y: b },
                Command::Close,
            ]);
        }
        let g = tessellate(request(commands, 0.001)).unwrap();
        let area: f32 = g
            .indices
            .chunks_exact(3)
            .map(|t| {
                let [a, b, c] = [
                    g.positions[t[0] as usize],
                    g.positions[t[1] as usize],
                    g.positions[t[2] as usize],
                ];
                let area = ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) / 2.0;
                assert!(area >= 0.0);
                area
            })
            .sum();
        assert!((area - 0.75).abs() < 0.00001);
    }
    #[test]
    fn excessive_flattening_is_rejected_before_tessellation() {
        let commands = vec![
            Command::MoveTo { x: 0.0, y: 0.0 },
            Command::BezierTo {
                c1x: 0.0,
                c1y: 1000.0,
                c2x: 1000.0,
                c2y: 1000.0,
                x: 1000.0,
                y: 0.0,
            },
        ];
        assert!(tessellate(request(commands, 0.00001))
            .err()
            .unwrap()
            .contains("512"));
    }
}
