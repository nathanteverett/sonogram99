//! Perspective wire sculptures, with separate soft afterimages. Each solid slowly bends into
//! another solid without coming apart, and the inner and outer solid take turns in each shell.

use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};

use crate::analysis::Frame;
use crate::canvas::{mix, Canvas};
use crate::glow::{per_frame, Field};
use crate::interaction::Rng;
use crate::modes::Mode;
use crate::palette::{Palette, SCREEN};

type Point = (f32, f32);
type Vec3 = [f32; 3];
/// The columns are where the x, y and z axes turn to.
type Matrix = [Vec3; 3];
/// A vertex of the old solid and the vertex of the new solid that it moves to.
type Joint = (usize, usize);

/// Seconds for one solid to become the next, at an average pace.
const MORPH: f32 = 6.0;
/// Seconds for the two solids to change shells, at an average pace.
const SWAP: f32 = 8.0;
/// Points along each wire while it bends, besides the corners of its routes.
const BENDS: usize = 4;
/// Angles to try the new solid at. The wires take the angle with the least travel.
const TRIES: usize = 12;

#[derive(Default)]
struct Motion {
    t: f32,
    bass: f32,
    treble: f32,
    energy: f32,
}

impl Motion {
    fn update(&mut self, f: &Frame, dt: f32) -> f32 {
        let dt = dt.clamp(0.0, 0.1);
        self.t += dt;
        let attack = if f.bass > self.bass { 20.0 } else { 3.5 };
        self.bass += (f.bass - self.bass) * (1.0 - (-attack * dt).exp());
        let high = ((f.band_avg(1500.0, 9000.0) - 0.15) * 2.0).clamp(0.0, 1.0);
        self.treble += (high - self.treble) * (1.0 - (-7.0 * dt).exp());
        self.energy += ((f.peak * 7.0).clamp(0.0, 1.0) - self.energy) * (1.0 - (-6.0 * dt).exp());
        dt
    }
}

/// Every sign combination of `v`. A zero coordinate has only one sign.
fn signs(v: Vec3) -> Vec<Vec3> {
    let mut out = vec![v];
    for axis in (0..3).filter(|&axis| v[axis] != 0.0) {
        let flipped: Vec<Vec3> = out.iter().map(|&p| { let mut p = p; p[axis] = -p[axis]; p }).collect();
        out.extend(flipped);
    }
    out
}

/// Each vertex with its coordinates moved round to every axis.
fn cycled(vertices: Vec<Vec3>) -> Vec<Vec3> {
    vertices.into_iter().flat_map(|[x, y, z]| [[x, y, z], [y, z, x], [z, x, y]]).collect()
}

/// The square of the distance.
fn distance(a: &Vec3, b: &Vec3) -> f32 {
    a.iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum()
}

fn length(v: &Vec3) -> f32 {
    v.iter().map(|v| v * v).sum::<f32>().sqrt()
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp3(a: &Vec3, b: &Vec3, t: f32) -> Vec3 {
    std::array::from_fn(|k| lerp(a[k], b[k], t))
}

/// Turns about x, then y, then z.
fn rotation([ax, ay, az]: Vec3) -> Matrix {
    let (sx, cx) = ax.sin_cos();
    let (sy, cy) = ay.sin_cos();
    let (sz, cz) = az.sin_cos();
    let turn = |v: Vec3| {
        let (y, z) = (v[1] * cx - v[2] * sx, v[1] * sx + v[2] * cx);
        let (x, z) = (v[0] * cy + z * sy, -v[0] * sy + z * cy);
        [x * cz - y * sz, x * sz + y * cz, z]
    };
    [turn([1.0, 0.0, 0.0]), turn([0.0, 1.0, 0.0]), turn([0.0, 0.0, 1.0])]
}

fn apply(m: &Matrix, v: Vec3) -> Vec3 {
    std::array::from_fn(|k| m[0][k] * v[0] + m[1][k] * v[1] + m[2][k] * v[2])
}

/// A solid as its vertices and the edges between them.
#[derive(Clone)]
struct Mesh {
    vertices: Vec<Vec3>,
    edges: Vec<(usize, usize)>,
}

impl Mesh {
    /// Joins nearest neighbors, which gives the edges of a solid whose edges all have one
    /// length. Every vertex moves onto the unit sphere. Construct once, not each frame.
    fn regular(mut vertices: Vec<Vec3>) -> Self {
        let radius = length(&vertices[0]);
        for v in &mut vertices { for axis in v { *axis /= radius; } }
        let mut nearest = f32::INFINITY;
        for i in 0..vertices.len() {
            for j in i + 1..vertices.len() { nearest = nearest.min(distance(&vertices[i], &vertices[j])); }
        }
        let mut edges = Vec::new();
        for i in 0..vertices.len() {
            for j in i + 1..vertices.len() {
                if distance(&vertices[i], &vertices[j]) < nearest * 1.01 { edges.push((i, j)); }
            }
        }
        Self { vertices, edges }
    }

    fn turned(&self, m: &Matrix) -> Self {
        Self { vertices: self.vertices.iter().map(|&v| apply(m, v)).collect(), edges: self.edges.clone() }
    }
}

/// The solids that the sculptures become. The first three make the opening sculpture.
fn solids() -> Vec<Mesh> {
    let phi = (1.0 + 5.0f32.sqrt()) / 2.0;
    let even = |v: &Vec3| v[0] * v[1] * v[2] > 0.0;
    let mut dodecahedron = signs([1.0; 3]);
    dodecahedron.extend(cycled(signs([0.0, 1.0 / phi, phi])));
    // Two tetrahedra through each other. Their edges cross at the middle of each cube face,
    // and a vertex there joins the two, so wires can travel from one to the other.
    let mut star = Mesh { vertices: signs([1.0; 3]), edges: Vec::new() };
    let faces = cycled(signs([1.0, 0.0, 0.0]));
    for (corner, v) in star.vertices.clone().iter().enumerate() {
        for axis in 0..3 {
            let face = faces.iter().position(|f| f[axis] == v[axis]).unwrap();
            star.edges.push((corner, 8 + face));
        }
    }
    star.vertices.extend(faces);
    for v in &mut star.vertices { for axis in v { *axis /= 3.0f32.sqrt(); } }
    vec![
        Mesh::regular(cycled(signs([0.0, 1.0, phi]))),
        Mesh::regular(signs([1.0; 3])),
        Mesh::regular(signs([1.0; 3]).into_iter().filter(even).collect()),
        Mesh::regular(cycled(signs([1.0, 0.0, 0.0]))),
        Mesh::regular(dodecahedron),
        Mesh::regular(cycled(signs([1.0, 1.0, 0.0]))),
        Mesh::regular(cycled(signs([3.0, 1.0, 1.0]).into_iter().filter(even).collect())),
        star,
    ]
}

/// A mesh with the shortest routes along its edges, for planning a change.
struct Routes<'a> {
    mesh: &'a Mesh,
    neighbors: Vec<Vec<usize>>,
    /// The number of edges on the shortest route between every two vertices.
    hops: Vec<Vec<usize>>,
    /// The edge between two vertices, if there is one.
    edge: Vec<Vec<Option<usize>>>,
}

impl<'a> Routes<'a> {
    fn new(mesh: &'a Mesh) -> Self {
        let n = mesh.vertices.len();
        let (mut neighbors, mut edge) = (vec![Vec::new(); n], vec![vec![None; n]; n]);
        for (k, &(a, b)) in mesh.edges.iter().enumerate() {
            neighbors[a].push(b);
            neighbors[b].push(a);
            (edge[a][b], edge[b][a]) = (Some(k), Some(k));
        }
        let hops = (0..n).map(|start| {
            let mut hops = vec![usize::MAX; n];
            hops[start] = 0;
            let mut queue = VecDeque::from([start]);
            while let Some(v) = queue.pop_front() {
                for &w in &neighbors[v] {
                    if hops[w] == usize::MAX { hops[w] = hops[v] + 1; queue.push_back(w); }
                }
            }
            hops
        }).collect();
        Self { mesh, neighbors, hops, edge }
    }

    /// A shortest route along the edges. Where routes tie, each step goes to the vertex
    /// nearest the end, so the route stays close to the straight line.
    fn route(&self, from: usize, to: usize) -> Vec<usize> {
        let v = &self.mesh.vertices;
        let mut route = vec![from];
        while route[route.len() - 1] != to {
            let at = route[route.len() - 1];
            let next = self.neighbors[at].iter().copied()
                .filter(|&w| self.hops[w][to] < self.hops[at][to])
                .min_by(|&a, &b| distance(&v[a], &v[to]).total_cmp(&distance(&v[b], &v[to])))
                .expect("Every solid is connected");
            route.push(next);
        }
        route
    }

    /// The point `u` of the way along a route. All edges of a solid have one length.
    fn point(&self, route: &[usize], u: f32) -> Vec3 {
        let v = &self.mesh.vertices;
        let steps = route.len() - 1;
        if steps == 0 || u >= 1.0 {
            return v[route[steps]];
        }
        let x = u * steps as f32;
        let k = (x as usize).min(steps - 1);
        lerp3(&v[route[k]], &v[route[k + 1]], x - k as f32)
    }
}

/// One line of the sculpture. Its two ends are joints that it shares with other wires, so
/// the sculpture bends but never comes apart. Its points move from a route along the old
/// solid's edges to a route along the new solid's edges.
struct Wire {
    from: Vec<Vec3>,
    to: Vec<Vec3>,
    /// Whether the wire draws its edge of the old and of the new solid. Other wires lie on
    /// top of those edges there, so they fade in and out at the very start and end.
    owner: [bool; 2],
}

impl Wire {
    fn new(a: &Routes, from: &[usize], b: &Routes, to: &[usize], owner: [bool; 2]) -> Self {
        // Points at both routes' corners, so that the wire lies exactly on the edges at the ends.
        let mut stops: Vec<f32> = (0..=BENDS).map(|k| k as f32 / BENDS as f32).collect();
        for route in [from, to] {
            let steps = route.len() - 1;
            stops.extend((1..steps).map(|k| k as f32 / steps as f32));
        }
        stops.sort_by(f32::total_cmp);
        stops.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        Self {
            from: stops.iter().map(|&u| a.point(from, u)).collect(),
            to: stops.iter().map(|&u| b.point(to, u)).collect(),
            owner,
        }
    }

    fn settled(mesh: &Mesh) -> Vec<Self> {
        mesh.edges.iter().map(|&(a, b)| {
            let edge = vec![mesh.vertices[a], mesh.vertices[b]];
            Self { from: edge.clone(), to: edge, owner: [true; 2] }
        }).collect()
    }

    /// The wire `e` of the way through the change.
    fn points(&self, e: f32) -> impl Iterator<Item = Vec3> + '_ {
        self.from.iter().zip(&self.to).map(move |(s, t)| swing(s, t, e))
    }

    fn light(&self, e: f32) -> f32 {
        let start = if self.owner[0] { 1.0 } else { (e / 0.04).min(1.0) };
        let end = if self.owner[1] { 1.0 } else { ((1.0 - e) / 0.04).min(1.0) };
        start * end
    }

    /// How far the wire's points travel, on average.
    fn travel(&self) -> f32 {
        self.from.iter().zip(&self.to).map(|(s, t)| distance(s, t)).sum::<f32>() / self.from.len() as f32
    }
}

/// Moves a point round the center rather than straight through it, so the solid keeps its
/// volume while it changes.
fn swing(s: &Vec3, t: &Vec3, e: f32) -> Vec3 {
    let p = lerp3(s, t, e);
    let r = lerp(length(s), length(t), e);
    // Near the center the direction is unsure, so the point cuts across instead.
    let scale = r / length(&p).max(r * 0.3).max(1e-6);
    p.map(|v| v * scale)
}

/// The pair of joints, one from each list, with the lowest cost.
fn best(left: &[Joint], right: &[Joint], cost: impl Fn(Joint, Joint) -> f32) -> (Joint, Joint) {
    let pairs = left.iter().flat_map(|&j| right.iter().map(move |&k| (j, k)));
    pairs.min_by(|x, y| cost(x.0, x.1).total_cmp(&cost(y.0, y.1))).unwrap()
}

/// Plans the change from solid `a` to solid `b`. Returns the wires and how far they travel.
fn plan(a: &Mesh, b: &Mesh) -> (Vec<Wire>, f32) {
    let (ra, rb) = (Routes::new(a), Routes::new(b));
    let (va, vb) = (&a.vertices, &b.vertices);
    // The nearest vertices join first. Each vertex left over joins its nearest vertex on the
    // other solid, so vertices split apart or merge when the solids have different counts.
    let mut pairs: Vec<(f32, usize, usize)> = (0..va.len())
        .flat_map(|i| (0..vb.len()).map(move |j| (i, j)))
        .map(|(i, j)| (distance(&va[i], &vb[j]), i, j))
        .collect();
    pairs.sort_by(|x, y| x.0.total_cmp(&y.0));
    let (mut used_a, mut used_b) = (vec![false; va.len()], vec![false; vb.len()]);
    let mut joints = Vec::new();
    for (_, i, j) in pairs {
        if !used_a[i] && !used_b[j] {
            (used_a[i], used_b[j]) = (true, true);
            joints.push((i, j));
        }
    }
    let nearest = |p: &Vec3, to: &[Vec3]| {
        (0..to.len()).min_by(|&i, &j| distance(p, &to[i]).total_cmp(&distance(p, &to[j]))).unwrap()
    };
    joints.extend((0..va.len()).filter(|&i| !used_a[i]).map(|i| (i, nearest(&va[i], vb))));
    joints.extend((0..vb.len()).filter(|&j| !used_b[j]).map(|j| (nearest(&vb[j], va), j)));
    let (mut on_a, mut on_b) = (vec![Vec::new(); va.len()], vec![Vec::new(); vb.len()]);
    for &joint in &joints {
        on_a[joint.0].push(joint);
        on_b[joint.1].push(joint);
    }

    // Each old edge bends along the shortest route between its joints' new vertices.
    let mut drawn = vec![false; b.edges.len()];
    let mut wires = Vec::new();
    for &(a1, a2) in &a.edges {
        let (j1, j2) = best(&on_a[a1], &on_a[a2], |p, q| rb.hops[p.1][q.1] as f32 * 8.0 + distance(&vb[p.1], &vb[q.1]));
        let route = rb.route(j1.1, j2.1);
        let edge = if route.len() == 2 { rb.edge[route[0]][route[1]] } else { None };
        let owner = edge.is_some_and(|k| !std::mem::replace(&mut drawn[k], true));
        wires.push(Wire::new(&ra, &[a1, a2], &rb, &route, [true, owner]));
    }
    // Each new edge that no old edge became grows out of a route along the old edges.
    for (k, &(b1, b2)) in b.edges.iter().enumerate() {
        if drawn[k] { continue; }
        let (j1, j2) = best(&on_b[b1], &on_b[b2], |p, q| ra.hops[p.0][q.0] as f32 * 8.0 + distance(&va[p.0], &va[q.0]));
        wires.push(Wire::new(&ra, &ra.route(j1.0, j2.0), &rb, &[b1, b2], [false, true]));
    }
    let travel = wires.iter().map(Wire::travel).sum();
    (wires, travel)
}

/// The sizes of the outer and the inner shell, as fractions of the shorter side of the
/// picture. The two solids take turns in each shell.
const OUTER: f32 = 0.36;
const INNER: f32 = 0.15;
/// How far a solid may reach from the center, as a fraction of the shorter side.
const ROOM: f32 = 0.46;

/// Starts and stops gently.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Lets `r` grow freely up to most of `room`, then ever more slowly, so it never reaches it.
fn fit(r: f32, room: f32) -> f32 {
    let knee = room * 0.7;
    if r <= knee { r } else { knee + (room - knee) * ((r - knee) / (room - knee)).tanh() }
}

struct Piece {
    shape: usize,
    /// The current solid, at the angle the piece took it on.
    mesh: Mesh,
    wires: Vec<Wire>,
    /// How far the change into `shape` has gone, 0 to 1.
    morph: f32,
    /// How long the piece keeps its shape after a change, in seconds at an average pace.
    hold: f32,
    /// Size against its place, before and after the change.
    grow: [f32; 2],
    /// How much the piece swells (above 0) or shrinks (below 0) halfway through a change.
    swell: f32,
    angle: f32,
    spin: f32,
    spin_to: f32,
    hue: f32,
    /// The size of the shell the piece leaves and of the shell it moves to.
    shell: [f32; 2],
}

impl Piece {
    fn size(&self) -> f32 {
        let e = ease(self.morph);
        lerp(self.grow[0], self.grow[1], e) * (1.0 + self.swell * (PI * e).sin())
    }

    /// The size of the piece's shell, `e` of the way through a swap.
    fn shell(&self, e: f32) -> f32 {
        lerp(self.shell[0], self.shell[1], e)
    }

    /// Starts to bend into another solid, with a new size and spin.
    fn reshape(&mut self, solids: &[Mesh], rng: &mut Rng) {
        let mut next = rng.below(solids.len() - 1);
        if next >= self.shape { next += 1; }
        let (mesh, wires, _) = (0..TRIES)
            .map(|_| {
                let mesh = solids[next].turned(&rotation([rng.unit(), rng.unit(), rng.unit()].map(|v| v * TAU)));
                let (wires, travel) = plan(&self.mesh, &mesh);
                (mesh, wires, travel)
            })
            .min_by(|x, y| x.2.total_cmp(&y.2))
            .unwrap();
        (self.shape, self.mesh, self.wires) = (next, mesh, wires);
        self.morph = 0.0;
        self.hold = 8.0 + rng.unit() * 7.0;
        self.grow = [self.grow[1], 0.8 + rng.unit() * 0.4];
        self.swell = (0.12 + rng.unit() * 0.18) * if rng.unit() < 0.5 { -1.0 } else { 1.0 };
        let turn = if rng.unit() < 0.3 { -1.0 } else { 1.0 };
        self.spin_to = self.spin_to.signum() * turn * (0.5 + rng.unit() * 0.8);
    }
}

pub struct Wireframe {
    field: Field,
    motion: Motion,
    solids: Vec<Mesh>,
    pieces: Vec<Piece>,
    rng: Rng,
    /// How far the two solids have gone in changing shells, 0 to 1.
    swap: f32,
    until_swap: f32,
}

impl Default for Wireframe {
    fn default() -> Self {
        let mut mode = Self {
            field: Field::default(), motion: Motion::default(), solids: solids(), pieces: Vec::new(),
            rng: Rng::new(99), swap: 1.0, until_swap: 0.0,
        };
        mode.start();
        mode
    }
}

impl Wireframe {
    /// Returns to the opening sculpture: an icosahedron round a cube. The cube changes first
    /// and the icosahedron last. The random choices go on, so each visit develops differently.
    fn start(&mut self) {
        let opening = [(0, 1.0, 0.2, 15.0, OUTER), (1, -1.25, 0.8, 11.0, INNER)];
        self.pieces = opening.iter().map(|&(shape, spin, hue, hold, shell)| Piece {
            shape, mesh: self.solids[shape].clone(), wires: Wire::settled(&self.solids[shape]), morph: 1.0,
            hold: hold + self.rng.unit() * 3.0, grow: [1.0; 2], swell: 0.0,
            angle: 0.0, spin, spin_to: spin, hue, shell: [shell; 2],
        }).collect();
        self.swap = 1.0;
        self.until_swap = 14.0 + self.rng.unit() * 6.0;
    }

    /// The inner solid starts to grow out through the outer one, which shrinks into the middle.
    fn swap_shells(&mut self) {
        for piece in &mut self.pieces {
            let shell = if piece.shell[1] == OUTER { INNER } else { OUTER };
            piece.shell = [piece.shell[1], shell];
        }
        self.swap = 0.0;
        self.until_swap = 16.0 + self.rng.unit() * 10.0;
    }

    /// Louder music hurries every change, and a beat starts a change that is nearly due.
    fn advance(&mut self, f: &Frame, dt: f32) {
        let m = &self.motion;
        let pace = 0.6 + 0.8 * m.energy;
        let beat = f.music.beat > 0.0;
        self.swap = (self.swap + dt * pace / SWAP).min(1.0);
        if self.swap >= 1.0 {
            self.until_swap -= dt * pace;
        }
        let turn = dt * (0.5 + m.bass * 0.36);
        // Only one piece changes on each frame, so changes do not pile up on one beat.
        let mut changed = false;
        for piece in &mut self.pieces {
            piece.angle += turn * piece.spin;
            piece.spin += (piece.spin_to - piece.spin) * (1.0 - (-0.6 * dt).exp());
            if piece.morph < 1.0 {
                piece.morph = (piece.morph + dt * pace / MORPH).min(1.0);
                // The bent wires now lie on the new edges, so plain edges look the same.
                if piece.morph >= 1.0 { piece.wires = Wire::settled(&piece.mesh); }
                continue;
            }
            piece.hold -= dt * pace;
            if !changed && (piece.hold <= 0.0 || (beat && piece.hold < 2.0)) {
                piece.reshape(&self.solids, &mut self.rng);
                changed = true;
            }
        }
        if self.swap >= 1.0 && (self.until_swap <= 0.0 || (beat && self.until_swap < 4.0)) {
            self.swap_shells();
        }
    }
}

/// Rotation followed by perspective; the camera stays outside the solids.
fn project(v: Vec3, turn: &Matrix, scale: f32, center: Point, size: f32) -> Vec3 {
    let p = apply(turn, v).map(|v| v * scale);
    let perspective = 4.2 / (4.2 - p[2]);
    [center.0 + p[0] * size * perspective, center.1 + p[1] * size * perspective, p[2]]
}

impl Mode for Wireframe {
    fn name(&self) -> &'static str { "WIREFRAME" }

    fn reset(&mut self, c: &mut Canvas) {
        c.fill(SCREEN);
        self.field.clear();
        self.motion = Motion::default();
        self.start();
    }

    fn draw(&mut self, c: &mut Canvas, f: &Frame, pal: &Palette, dt: f32) {
        let dt = self.motion.update(f, dt);
        self.advance(f, dt);
        let m = &self.motion;
        let size = c.w.min(c.h) as f32;
        let center = (c.w as f32 * 0.5, c.h as f32 * 0.5);
        self.field.fit(c.w, c.h);
        self.field.warp(center, center, per_frame(1.003, dt), dt * 0.025, per_frame(0.982, dt), true);
        let swap = ease(self.swap);
        let mut lines = Vec::with_capacity(400);
        for piece in &self.pieces {
            let shell = piece.shell(swap);
            // Bass pulses the outer piece and treble the inner one. Every beat gives a kick.
            let large = ((shell - INNER) / (OUTER - INNER)).clamp(0.0, 1.0);
            let pulse = 1.0 + lerp(m.treble * 0.16, m.bass * 0.14, large) + f.music.pulse * 0.1;
            let radius = fit(shell * piece.size() * pulse, ROOM);
            let turn = rotation([piece.angle * 0.71, piece.angle, piece.angle * 0.39]);
            let e = ease(piece.morph);
            for wire in &piece.wires {
                let light = wire.light(e);
                if light <= 0.0 { continue; }
                let points: Vec<Vec3> = wire.points(e).map(|v| project(v, &turn, radius, center, size)).collect();
                for pair in points.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    let depth = (a[2] + b[2]) * 0.5;
                    let color = piece.hue + 0.10 * (m.t * 0.25 + depth * 3.0).sin();
                    lines.push((depth, a, b, color, light));
                    self.field.line(a[0], a[1], b[0], b[1], (size * 0.007).max(1.0), color, color,
                        m.energy * dt * 60.0 * 0.14 * light);
                }
            }
        }
        self.field.resolve_at(c, pal, 0.32);
        // Far edges first, then the luminous near edges. Keep every face transparent.
        lines.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (depth, a, b, hue, share) in lines {
            let front = ((depth + 0.3) / 0.6).clamp(0.0, 1.0);
            let color = mix(pal.glow(hue), pal.skin.peak, front * 0.20);
            let light = m.energy * (0.40 + front * 0.6) * share;
            c.line(a[0], a[1], b[0], b[1], color, light * 0.12, 3, false);
            c.line(a[0], a[1], b[0], b[1], color, light, 1, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Analyzer;
    use crate::canvas::{pack, unpack};

    #[test]
    fn solids_are_connected_with_even_edges_and_finite_projection() {
        for (mesh, count) in solids().iter().zip([30, 12, 6, 12, 30, 24, 18, 24]) {
            assert_eq!(mesh.edges.len(), count);
            assert!(Routes::new(mesh).hops.iter().flatten().all(|&h| h < usize::MAX), "Every solid is connected");
            let edge = |&(a, b): &(usize, usize)| distance(&mesh.vertices[a], &mesh.vertices[b]);
            assert!(mesh.edges.iter().all(|e| (edge(e) - edge(&mesh.edges[0])).abs() < 1e-4), "Every edge has one length");
            for &v in &mesh.vertices {
                assert!(length(&v) < 1.0 + 1e-4);
                for step in 0..60 {
                    let turn = rotation([step as f32 * 0.071, step as f32 * 0.1, step as f32 * 0.039]);
                    let p = project(v, &turn, ROOM, (320.0, 170.0), 340.0);
                    assert!(p.iter().all(|v| v.is_finite()));
                }
            }
        }
    }

    #[test]
    fn wires_bend_from_one_solid_to_the_next_without_coming_apart() {
        let solids = solids();
        let on_edge = |p: &Vec3, mesh: &Mesh| mesh.edges.iter().any(|&(a, b)| {
            let (a, b) = (mesh.vertices[a], mesh.vertices[b]);
            let along: Vec3 = std::array::from_fn(|k| b[k] - a[k]);
            let u = (0..3).map(|k| (p[k] - a[k]) * along[k]).sum::<f32>() / distance(&a, &b);
            distance(p, &lerp3(&a, &b, u.clamp(0.0, 1.0))) < 1e-8
        });
        let mut rng = Rng::new(7);
        for from in &solids {
            for to in &solids {
                let to = to.turned(&rotation([rng.unit(), rng.unit(), rng.unit()].map(|v| v * TAU)));
                let (wires, _) = plan(from, &to);
                for (end, mesh) in [(0, from), (1, &to)] {
                    let e = end as f32;
                    for wire in &wires {
                        assert!(wire.points(e).all(|p| on_edge(&p, mesh)), "At the ends, every wire lies on an edge");
                        assert!(wire.owner[end] || wire.light(e) == 0.0, "Only one wire lights each edge");
                    }
                    for &(a, b) in &mesh.edges {
                        let ends = |w: &Wire| {
                            let p: Vec<Vec3> = w.points(e).collect();
                            let (first, last) = (p[0], p[p.len() - 1]);
                            let at = |p: &Vec3, v: usize| distance(p, &mesh.vertices[v]) < 1e-8;
                            (at(&first, a) && at(&last, b)) || (at(&first, b) && at(&last, a))
                        };
                        assert_eq!(wires.iter().filter(|w| w.owner[end] && ends(w)).count(), 1, "One wire draws each edge");
                    }
                }
                for e in [0.3, 0.5, 0.8] {
                    let ends: Vec<Vec3> = wires.iter().flat_map(|w| {
                        let p: Vec<Vec3> = w.points(e).collect();
                        assert!(p.iter().flatten().all(|v| v.is_finite()));
                        [p[0], p[p.len() - 1]]
                    }).collect();
                    for p in &ends {
                        let shared = ends.iter().filter(|q| distance(p, q) < 1e-10).count();
                        assert!(shared >= 3, "A joint holds {shared} wire ends, not 3 or more");
                    }
                }
            }
        }
    }

    #[test]
    fn sculptures_change_shape_size_and_shell_while_staying_on_screen() {
        let mut mode = Wireframe::default();
        let mut c = Canvas::new(160, 90);
        let pal = Palette::new(3);
        let mut a = Analyzer::new();
        a.frame.peak = 0.1;
        a.frame.bass = 0.6;
        a.frame.spec.fill(0.5);
        let shapes: Vec<usize> = mode.pieces.iter().map(|p| p.shape).collect();
        let (mut changed, mut swaps, mut smallest, mut largest) = ([false; 2], 0, f32::INFINITY, 0.0f32);
        for _ in 0..60 * 90 {
            let before = mode.swap;
            mode.draw(&mut c, &a.frame, &pal, 1.0 / 60.0);
            if mode.swap < before { swaps += 1; }
            let swap = ease(mode.swap);
            let shells: Vec<f32> = mode.pieces.iter().map(|p| p.shell(swap)).collect();
            assert!((shells[0] + shells[1] - OUTER - INNER).abs() < 1e-5, "One piece is outside while the other is inside");
            for (k, (piece, shell)) in mode.pieces.iter().zip(shells).enumerate() {
                changed[k] |= piece.shape != shapes[k];
                let size = piece.size();
                (smallest, largest) = (smallest.min(size), largest.max(size));
                // The loudest bass and the strongest beat at once.
                let reach = fit(shell * size * 1.24, ROOM);
                assert!(reach < ROOM, "A piece reaches {reach} from the middle");
            }
        }
        assert_eq!(changed, [true; 2], "Both pieces must change shape in 90 seconds");
        assert!(swaps >= 2, "Only {swaps} swaps in 90 seconds");
        assert!(largest / smallest > 1.3, "Pieces must grow and shrink: {smallest} to {largest}");
        mode.reset(&mut c);
        assert_eq!(mode.pieces.iter().map(|p| p.shape).collect::<Vec<_>>(), [0, 1], "Reset returns to the opening sculpture");
        assert_eq!(mode.pieces.iter().map(|p| p.shell).collect::<Vec<_>>(), [[OUTER; 2], [INNER; 2]]);
    }

    #[test]
    fn wireframe_reacts_to_audio_moves_and_fades() {
        let (mut quiet, mut loud) = (Wireframe::default(), Wireframe::default());
        let (mut a, mut b) = (Canvas::new(120, 90), Canvas::new(120, 90));
        let pal = Palette::new(3);
        let (mut low, mut high) = (Analyzer::new(), Analyzer::new());
        quiet.draw(&mut a, &low.frame, &pal, 1.0 / 60.0);
        assert!(a.px.iter().all(|&v| v == pack(SCREEN)), "Silence starts dark");
        quiet.reset(&mut a);
        low.frame.peak = 0.3;
        high.frame.peak = 0.3;
        high.frame.bass = 0.9;
        high.frame.spec.fill(0.8);
        for _ in 0..45 {
            quiet.draw(&mut a, &low.frame, &pal, 1.0 / 60.0);
            loud.draw(&mut b, &high.frame, &pal, 1.0 / 60.0);
        }
        assert_ne!(a.px, b.px, "Bass and treble must change the image");
        let first = b.px.clone();
        for _ in 0..45 { loud.draw(&mut b, &high.frame, &pal, 1.0 / 60.0); }
        assert_ne!(first, b.px, "Geometry must animate over time");
        let light = |c: &Canvas| -> u64 {
            c.px.iter().map(|&v| unpack(v).iter().map(|&v| u64::from(v)).sum::<u64>()).sum()
        };
        let active = light(&b);
        let silence = Analyzer::new();
        for _ in 0..480 { loud.draw(&mut b, &silence.frame, &pal, 1.0 / 60.0); }
        assert!(light(&b) < active / 2, "Background must fade after playback ends");
        for (w, h) in [(90, 120), (240, 100), (1, 1)] {
            b.resize(w, h);
            loud.reset(&mut b);
            loud.draw(&mut b, &high.frame, &pal, 1.0 / 30.0);
        }
        loud.reset(&mut b);
        assert!(b.px.iter().all(|&v| v == pack(SCREEN)));
    }
}
