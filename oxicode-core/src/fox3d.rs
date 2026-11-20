use crate::theme::Rgb;

#[derive(Debug, Clone, Copy)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3 {
    const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }

    fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    fn norm(self) -> Self {
        let len = (self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        if len > 1e-6 {
            Self::new(self.x / len, self.y / len, self.z / len)
        } else {
            Self::new(0.0, 0.0, 1.0)
        }
    }

    fn rotate(self, yaw: f32, pitch: f32) -> Self {
        // Yaw around Y axis, then pitch around X axis
        let (sy, cy) = yaw.sin_cos();
        let x1 = self.x * cy + self.z * sy;
        let z1 = -self.x * sy + self.z * cy;
        let (sp, cp) = pitch.sin_cos();
        let y2 = self.y * cp - z1 * sp;
        let z2 = self.y * sp + z1 * cp;
        Self::new(x1, y2, z2)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FacetMat {
    Orange,
    OrangeDeep,
    Beige,
    EarInner,
    EarTip,
    Nose,
}

#[derive(Debug, Clone, Copy)]
struct Tri {
    a: Vec3,
    b: Vec3,
    c: Vec3,
    mat: FacetMat,
}

const fn tri_oriented(a: [f32; 3], b: [f32; 3], c: [f32; 3], mat: FacetMat, front: bool) -> Tri {
    let va = Vec3::new(a[0], a[1], a[2]);
    let vb = Vec3::new(b[0], b[1], b[2]);
    let vc = Vec3::new(c[0], c[1], c[2]);
    let ux = vb.x - va.x;
    let uy = vb.y - va.y;
    let vx = vc.x - va.x;
    let vy = vc.y - va.y;
    let nz = ux * vy - uy * vx;
    let want_pos = front;
    if (want_pos && nz < 0.0) || (!want_pos && nz > 0.0) {
        Tri { a: va, b: vc, c: vb, mat }
    } else {
        Tri { a: va, b: vb, c: vc, mat }
    }
}

const fn tri(a: [f32; 3], b: [f32; 3], c: [f32; 3], mat: FacetMat) -> Tri {
    tri_oriented(a, b, c, mat, true)
}

const fn tri_back(a: [f32; 3], b: [f32; 3], c: [f32; 3], mat: FacetMat) -> Tri {
    tri_oriented(a, b, c, mat, false)
}

// Key vertices for symmetric Origami Kitsune head
const V_CROWN: [f32; 3] = [0.0, 0.46, 0.16];
const V_BROW: [f32; 3] = [0.0, 0.14, 0.48];
const V_SNOUT_TOP: [f32; 3] = [0.0, -0.30, 0.78];
const V_NOSE_TIP: [f32; 3] = [0.0, -0.35, 0.89];
const V_NOSE_BOT: [f32; 3] = [0.0, -0.47, 0.76];
const V_CHIN: [f32; 3] = [0.0, -0.60, 0.42];
const V_BACK_TOP: [f32; 3] = [0.0, 0.38, -0.36];

const fn fox_mesh() -> [Tri; 38] {
    // Left side vertices (negative X), right side mirrors +X with flipped winding
    let ear_l_tip = [-0.74, 0.96, -0.04];
    let ear_r_tip = [0.74, 0.96, -0.04];
    let ear_l_mid_in = [-0.46, 0.64, 0.06];
    let ear_r_mid_in = [0.46, 0.64, 0.06];
    let ear_l_mid_out = [-0.72, 0.56, 0.00];
    let ear_r_mid_out = [0.72, 0.56, 0.00];
    let ear_l_in = [-0.24, 0.40, 0.12];
    let ear_r_in = [0.24, 0.40, 0.12];
    let ear_l_out = [-0.64, 0.18, 0.08];
    let ear_r_out = [0.64, 0.18, 0.08];

    let temple_l = [-0.54, 0.20, 0.26];
    let temple_r = [0.54, 0.20, 0.26];
    let cheek_l_out = [-0.96, -0.14, 0.04];
    let cheek_r_out = [0.96, -0.14, 0.04];
    let cheek_l_front = [-0.46, -0.20, 0.42];
    let cheek_r_front = [0.46, -0.20, 0.42];
    let jaw_l = [-0.52, -0.46, 0.20];
    let jaw_r = [0.52, -0.46, 0.20];
    let nose_l = [-0.15, -0.26, 0.77];
    let nose_r = [0.15, -0.26, 0.77];
    let back_l = [-0.56, 0.04, -0.30];
    let back_r = [0.56, 0.04, -0.30];

    [
        // Ears: copper tips & outer rim, cream inner triangles, solid bridge to crown
        tri(ear_l_tip, ear_l_mid_in, ear_l_mid_out, FacetMat::EarTip),
        tri(ear_r_tip, ear_r_mid_out, ear_r_mid_in, FacetMat::EarTip),
        tri(ear_l_mid_in, ear_l_in, ear_l_mid_out, FacetMat::EarInner),
        tri(ear_r_mid_in, ear_r_mid_out, ear_r_in, FacetMat::EarInner),
        tri(ear_l_mid_out, ear_l_in, ear_l_out, FacetMat::EarInner),
        tri(ear_r_mid_out, ear_r_out, ear_r_in, FacetMat::EarInner),
        tri(ear_l_out, temple_l, ear_l_mid_out, FacetMat::Orange),
        tri(ear_r_out, ear_r_mid_out, temple_r, FacetMat::Orange),
        tri(ear_l_in, ear_l_out, temple_l, FacetMat::Orange),
        tri(ear_r_in, temple_r, ear_r_out, FacetMat::Orange),
        tri(ear_l_out, cheek_l_out, temple_l, FacetMat::Orange),
        tri(ear_r_out, temple_r, cheek_r_out, FacetMat::Orange),
        // Ear backs
        tri_back(ear_l_tip, ear_l_mid_out, ear_l_mid_in, FacetMat::OrangeDeep),
        tri_back(ear_r_tip, ear_r_mid_in, ear_r_mid_out, FacetMat::OrangeDeep),
        tri_back(ear_l_mid_out, ear_l_out, ear_l_in, FacetMat::OrangeDeep),
        tri_back(ear_r_mid_out, ear_r_in, ear_r_out, FacetMat::OrangeDeep),

        // Forehead & brow bridge
        tri(V_CROWN, ear_l_in, temple_l, FacetMat::Orange),
        tri(V_CROWN, temple_r, ear_r_in, FacetMat::Orange),
        tri(V_CROWN, temple_l, V_BROW, FacetMat::Orange),
        tri(V_CROWN, V_BROW, temple_r, FacetMat::Orange),

        // Upper face / eye planes
        tri(temple_l, cheek_l_out, cheek_l_front, FacetMat::Orange),
        tri(temple_r, cheek_r_front, cheek_r_out, FacetMat::Orange),
        tri(V_BROW, temple_l, cheek_l_front, FacetMat::Orange),
        tri(V_BROW, cheek_r_front, temple_r, FacetMat::Orange),

        // Snout bridge
        tri(V_BROW, cheek_l_front, V_SNOUT_TOP, FacetMat::Orange),
        tri(V_BROW, V_SNOUT_TOP, cheek_r_front, FacetMat::Orange),

        // Beige cheeks & muzzle sides
        tri(cheek_l_out, jaw_l, cheek_l_front, FacetMat::Beige),
        tri(cheek_r_out, cheek_r_front, jaw_r, FacetMat::Beige),
        tri(cheek_l_front, jaw_l, V_SNOUT_TOP, FacetMat::Beige),
        tri(cheek_r_front, V_SNOUT_TOP, jaw_r, FacetMat::Beige),
        tri(jaw_l, V_CHIN, V_SNOUT_TOP, FacetMat::Beige),
        tri(jaw_r, V_SNOUT_TOP, V_CHIN, FacetMat::Beige),

        // Nose diamond
        tri(nose_l, V_NOSE_BOT, V_NOSE_TIP, FacetMat::Nose),
        tri(nose_r, V_NOSE_TIP, V_NOSE_BOT, FacetMat::Nose),

        // Back / side closure so wide turns remain solid
        tri_back(V_CROWN, V_BACK_TOP, ear_l_in, FacetMat::OrangeDeep),
        tri_back(V_CROWN, ear_r_in, V_BACK_TOP, FacetMat::OrangeDeep),
        tri_back(cheek_l_out, back_l, jaw_l, FacetMat::OrangeDeep),
        tri_back(cheek_r_out, jaw_r, back_r, FacetMat::OrangeDeep),
    ]
}

const FOX_TRIS: [Tri; 38] = fox_mesh();

/// Live 3D head pose and animation state for the OXICODE Origami Fox mascot.
#[derive(Debug, Clone)]
pub struct FoxState {
    pub yaw: f32,
    pub pitch: f32,
    pub target_yaw: f32,
    pub target_pitch: f32,
    pub blink_timer: f32,
    pub is_blinking: bool,
    pub thinking_phase: f32,
}

impl Default for FoxState {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            target_yaw: 0.0,
            target_pitch: 0.0,
            blink_timer: 3.4,
            is_blinking: false,
            thinking_phase: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsciiCell {
    pub ch: char,
    pub fg: Rgb,
}

/// Single half-block cell for the static Antigravity-style pixel Fox logo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelCell {
    pub ch: char,
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
}

/// Renders a static 16x12 pixel-art Origami Fox emblem into 6 terminal rows (`16` columns wide).
/// Designed specifically for the Antigravity UI style: fixed straight gaze, no cursor/typing tracking.
pub fn render_pixel_fox_emblem(monochrome: bool, legacy_ascii: bool) -> Vec<Vec<PixelCell>> {
    const ART: [&[u8; 16]; 12] = [
        b".T............T.",
        b".TT..........TT.",
        b".TIT........TIT.",
        b".TIIT......TIIT.",
        b".TIIOOOOOOOOIIT.",
        b".OOOOOOOOOOOOOO.",
        b".OOGEOOOOOOGEOO.",
        b".OOEEOOOOOOEEOO.",
        b".CCOOOOOOOOOOCC.",
        b"..CCCCOOOOCCCC..",
        b"....CCCNNCCC....",
        b"......CCCC......",
    ];

    let pixel_color = |code: u8, x: usize, y: usize| -> Option<Rgb> {
        if code == b'.' {
            return None;
        }
        let u = x as f32 / 15.0;
        let v = y as f32 / 11.0;
        if monochrome {
            let luma = match code {
                b'T' => (196.0 - u * 18.0) as u8,
                b'O' => (224.0 - u * 16.0 - v * 20.0) as u8,
                b'I' => 246,
                b'C' => 250,
                b'E' => 26,
                b'G' => 236,
                b'N' => 42,
                _ => 210,
            };
            return Some(Rgb::new(luma, luma, luma));
        }

        // Warm prism gradient inspired by Antigravity's emblem while keeping fox fur & cream palette
        let rgb = match code {
            b'T' => Rgb::new(
                (252.0 - u * 30.0) as u8,
                (162.0 - u * 78.0) as u8,
                (64.0 + u * 44.0) as u8,
            ),
            b'O' => Rgb::new(
                (254.0 - u * 28.0 - v * 12.0) as u8,
                (168.0 - u * 74.0 - v * 22.0).max(58.0) as u8,
                (62.0 + u * 48.0 + v * 10.0) as u8,
            ),
            b'I' => Rgb::new(255, (226.0 - u * 18.0) as u8, (158.0 + u * 18.0) as u8),
            b'C' => Rgb::new(
                (252.0 - v * 8.0) as u8,
                (236.0 - u * 12.0) as u8,
                (210.0 + u * 10.0) as u8,
            ),
            b'E' => Rgb::new(28, 32, 54),
            b'G' => Rgb::new(146, 212, 255),
            b'N' => Rgb::new(44, 42, 62),
            _ => Rgb::new(230, 140, 80),
        };
        Some(rgb)
    };

    let mut out = Vec::with_capacity(6);
    for row_pair in 0..6 {
        let y_top = row_pair * 2;
        let y_bot = y_top + 1;
        let mut cells = Vec::with_capacity(16);
        for (x, (&top_code, &bot_code)) in ART[y_top].iter().zip(ART[y_bot].iter()).enumerate() {
            let top = pixel_color(top_code, x, y_top);
            let bot = pixel_color(bot_code, x, y_bot);
            let cell = match (top, bot) {
                (None, None) => PixelCell {
                    ch: ' ',
                    fg: None,
                    bg: None,
                },
                (Some(t), None) => PixelCell {
                    ch: if legacy_ascii { '^' } else { '▀' },
                    fg: Some(t),
                    bg: None,
                },
                (None, Some(b)) => PixelCell {
                    ch: if legacy_ascii { '_' } else { '▄' },
                    fg: Some(b),
                    bg: None,
                },
                (Some(t), Some(b)) => {
                    if t.r == b.r && t.g == b.g && t.b == b.b {
                        PixelCell {
                            ch: if legacy_ascii { '#' } else { '█' },
                            fg: Some(t),
                            bg: None,
                        }
                    } else if legacy_ascii {
                        PixelCell {
                            ch: '#',
                            fg: Some(t),
                            bg: None,
                        }
                    } else {
                        PixelCell {
                            ch: '▀',
                            fg: Some(t),
                            bg: Some(b),
                        }
                    }
                }
            };
            cells.push(cell);
        }
        out.push(cells);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PixelKind {
    Empty,
    Fur,
    EyeIris,
    EyePupil,
    EyeGlint,
    Nose,
}

impl FoxState {
    /// Aims the fox smoothly toward `(cursor_x, cursor_y)` relative to the fox center.
    pub fn look_at(&mut self, dx: f32, dy: f32) {
        self.target_yaw = (dx * 0.85).clamp(-0.72, 0.72);
        self.target_pitch = (dy * 0.55).clamp(-0.36, 0.36);
    }

    /// Smoothly advances head rotation, idle blinking, and thinking motion.
    /// Returns `true` when the visual frame changed and needs a redraw.
    pub fn step(&mut self, dt_secs: f32, is_busy: bool) -> bool {
        let mut dirty = false;

        if is_busy {
            self.thinking_phase += dt_secs * 3.4;
            self.target_yaw = self.thinking_phase.sin() * 0.24;
            self.target_pitch = 0.06 + (self.thinking_phase * 1.7).cos() * 0.05;
            dirty = true;
        }

        let lerp = (dt_secs * 12.0).clamp(0.08, 0.45);
        let dyaw = self.target_yaw - self.yaw;
        let dpitch = self.target_pitch - self.pitch;
        if dyaw.abs() > 0.004 || dpitch.abs() > 0.004 {
            self.yaw += dyaw * lerp;
            self.pitch += dpitch * lerp;
            dirty = true;
        } else if (self.yaw - self.target_yaw).abs() > 0.0005
            || (self.pitch - self.target_pitch).abs() > 0.0005
        {
            self.yaw = self.target_yaw;
            self.pitch = self.target_pitch;
            dirty = true;
        }

        self.blink_timer -= dt_secs;
        if self.blink_timer <= 0.0 {
            self.is_blinking = !self.is_blinking;
            self.blink_timer = if self.is_blinking { 0.14 } else { 4.2 };
            dirty = true;
        }

        dirty
    }

    fn rasterize_buffer(
        &self,
        w: usize,
        h: usize,
        bg: Rgb,
        monochrome: bool,
    ) -> (Vec<Rgb>, Vec<PixelKind>, Vec<f32>) {
        let mut colors = vec![bg; w * h];
        let mut kinds = vec![PixelKind::Empty; w * h];
        let mut shades = vec![0.0f32; w * h];
        let mut zbuf = vec![f32::NEG_INFINITY; w * h];

        let light = Vec3::new(-0.35, 0.55, 0.80).norm();
        let eye_centers = [Vec3::new(-0.34, 0.07, 0.50), Vec3::new(0.34, 0.07, 0.50)];

        let scale_x = w as f32 * 0.46;
        let scale_y = h as f32 * 0.56;
        let cx = w as f32 * 0.5;
        let cy = h as f32 * 0.58;

        let project = |p: Vec3| -> (f32, f32, f32) {
            let persp = 1.0 + p.z * 0.22;
            (cx + p.x * scale_x * persp, cy - p.y * scale_y * persp, p.z)
        };

        for t in FOX_TRIS {
            let ra = t.a.rotate(self.yaw, self.pitch);
            let rb = t.b.rotate(self.yaw, self.pitch);
            let rc = t.c.rotate(self.yaw, self.pitch);

            let normal = rb.sub(ra).cross(rc.sub(ra)).norm();
            if normal.z <= 0.02 {
                continue;
            }

            let shade = 0.58 + 0.42 * normal.dot(light).clamp(0.0, 1.0);
            let base_col = shade_mat(t.mat, shade, monochrome);

            let (ax, ay, az) = project(ra);
            let (bx, by, bz) = project(rb);
            let (cxp, cyp, cz) = project(rc);

            let min_x = ax.min(bx).min(cxp).floor().max(0.0) as usize;
            let max_x = (ax.max(bx).max(cxp).ceil() as usize).min(w - 1);
            let min_y = ay.min(by).min(cyp).floor().max(0.0) as usize;
            let max_y = (ay.max(by).max(cyp).ceil() as usize).min(h - 1);

            let area = edge_fn(ax, ay, bx, by, cxp, cyp);
            if area.abs() < 1e-4 {
                continue;
            }

            for py in min_y..=max_y {
                for px in min_x..=max_x {
                    let sx = px as f32 + 0.5;
                    let sy = py as f32 + 0.5;
                    let w0 = edge_fn(bx, by, cxp, cyp, sx, sy) / area;
                    let w1 = edge_fn(cxp, cyp, ax, ay, sx, sy) / area;
                    let w2 = edge_fn(ax, ay, bx, by, sx, sy) / area;
                    if w0 >= -0.02 && w1 >= -0.02 && w2 >= -0.02 {
                        let z = w0 * az + w1 * bz + w2 * cz;
                        let idx = py * w + px;
                        if z > zbuf[idx] {
                            zbuf[idx] = z;
                            colors[idx] = base_col;
                            kinds[idx] = PixelKind::Fur;
                            shades[idx] = shade;
                        }
                    }
                }
            }
        }

        // Expressive cursor-tracking eyes
        let eye_r = (h as f32 * 0.125).max(1.8);
        let eye_rx = eye_r;
        let eye_ry = if self.is_blinking { 0.55 } else { eye_r };
        let gaze_x = (self.yaw * 1.1).clamp(-0.55, 0.55) * eye_rx * 0.55;
        let gaze_y = (self.pitch * 1.1).clamp(-0.45, 0.45) * eye_ry * 0.50;

        for ec in eye_centers {
            let re = ec.rotate(self.yaw, self.pitch);
            if re.z < 0.05 {
                continue;
            }
            let (ex, ey, ez) = project(re);
            let min_x = (ex - eye_rx - 1.0).floor().max(0.0) as usize;
            let max_x = ((ex + eye_rx + 1.0).ceil() as usize).min(w - 1);
            let min_y = (ey - eye_ry - 1.0).floor().max(0.0) as usize;
            let max_y = ((ey + eye_ry + 1.0).ceil() as usize).min(h - 1);

            for py in min_y..=max_y {
                for px in min_x..=max_x {
                    let dx = (px as f32 + 0.5 - ex) / eye_rx;
                    let dy = (py as f32 + 0.5 - ey) / eye_ry;
                    let dist = dx * dx + dy * dy;
                    if dist <= 1.0 {
                        let idx = py * w + px;
                        if ez + 0.08 >= zbuf[idx] {
                            zbuf[idx] = ez + 0.08;
                            if self.is_blinking {
                                colors[idx] = if monochrome {
                                    Rgb::new(48, 48, 48)
                                } else {
                                    Rgb::new(42, 46, 66)
                                };
                                kinds[idx] = PixelKind::EyePupil;
                                shades[idx] = 0.65;
                            } else {
                                let pdx = (px as f32 + 0.5 - (ex + gaze_x)) / (eye_rx * 0.52);
                                let pdy = (py as f32 + 0.5 - (ey + gaze_y)) / (eye_ry * 0.52);
                                let pdist = pdx * pdx + pdy * pdy;
                                if pdist <= 0.34 && dx < 0.15 && dy < -0.1 {
                                    colors[idx] = if monochrome {
                                        Rgb::new(250, 250, 250)
                                    } else {
                                        Rgb::new(235, 244, 255)
                                    };
                                    kinds[idx] = PixelKind::EyeGlint;
                                    shades[idx] = 1.0;
                                } else if pdist <= 1.0 {
                                    colors[idx] = if monochrome {
                                        Rgb::new(30, 30, 30)
                                    } else {
                                        Rgb::new(48, 56, 82)
                                    };
                                    kinds[idx] = PixelKind::EyePupil;
                                    shades[idx] = 0.72;
                                } else {
                                    colors[idx] = if monochrome {
                                        Rgb::new(212, 212, 212)
                                    } else {
                                        Rgb::new(118, 174, 232)
                                    };
                                    kinds[idx] = PixelKind::EyeIris;
                                    shades[idx] = 0.90;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Button nose projected from snout tip
        let rn = Vec3::new(0.0, -0.35, 0.84).rotate(self.yaw, self.pitch);
        if rn.z > 0.1 {
            let (nx, ny, nz) = project(rn);
            let nr = (h as f32 * 0.085).max(1.2);
            let min_x = (nx - nr - 1.0).floor().max(0.0) as usize;
            let max_x = ((nx + nr + 1.0).ceil() as usize).min(w - 1);
            let min_y = (ny - nr - 1.0).floor().max(0.0) as usize;
            let max_y = ((ny + nr + 1.0).ceil() as usize).min(h - 1);
            for py in min_y..=max_y {
                for px in min_x..=max_x {
                    let dx = (px as f32 + 0.5 - nx) / nr;
                    let dy = (py as f32 + 0.5 - ny) / (nr * 0.85);
                    if dx * dx + dy * dy <= 1.0 {
                        let idx = py * w + px;
                        if nz + 0.06 >= zbuf[idx] {
                            zbuf[idx] = nz + 0.06;
                            colors[idx] = if monochrome {
                                Rgb::new(52, 52, 52)
                            } else {
                                Rgb::new(56, 58, 76)
                            };
                            kinds[idx] = PixelKind::Nose;
                            shades[idx] = 0.85;
                        }
                    }
                }
            }
        }

        (colors, kinds, shades)
    }

    /// Renders a soft, supersampled Cline-style shaded ASCII Fox (`cols` x `rows` terminal cells).
    pub fn render_ascii_grid(&self, cols: usize, rows: usize) -> Vec<Vec<AsciiCell>> {
        self.render_ascii_grid_styled(cols, rows, false)
    }

    /// Renders the shaded ASCII Fox using either the classic orange/beige coat or crisp monochrome.
    pub fn render_ascii_grid_styled(
        &self,
        cols: usize,
        rows: usize,
        monochrome: bool,
    ) -> Vec<Vec<AsciiCell>> {
        let c = cols.max(16);
        let r = rows.max(8);
        let w = c * 2;
        let h = r * 4;
        let dummy_bg = Rgb::new(0, 0, 0);
        let (colors, kinds, shades) = self.rasterize_buffer(w, h, dummy_bg, monochrome);

        let mut out = Vec::with_capacity(r);
        for cy in 0..r {
            let mut row_cells = Vec::with_capacity(c);
            for cx in 0..c {
                let mut sum_r = 0u32;
                let mut sum_g = 0u32;
                let mut sum_b = 0u32;
                let mut sum_shade = 0.0f32;
                let mut lit = 0u32;
                let mut eye_iris = 0u32;
                let mut eye_pupil = 0u32;
                let mut eye_glint = 0u32;
                let mut nose = 0u32;

                for dy in 0..4 {
                    for dx in 0..2 {
                        let idx = (cy * 4 + dy) * w + (cx * 2 + dx);
                        let k = kinds[idx];
                        if k != PixelKind::Empty {
                            lit += 1;
                            let col = colors[idx];
                            sum_r += u32::from(col.r);
                            sum_g += u32::from(col.g);
                            sum_b += u32::from(col.b);
                            sum_shade += shades[idx];
                            match k {
                                PixelKind::EyeIris => eye_iris += 1,
                                PixelKind::EyePupil => eye_pupil += 1,
                                PixelKind::EyeGlint => eye_glint += 1,
                                PixelKind::Nose => nose += 1,
                                PixelKind::Fur | PixelKind::Empty => {}
                            }
                        }
                    }
                }

                if lit == 0 {
                    row_cells.push(AsciiCell {
                        ch: ' ',
                        fg: dummy_bg,
                    });
                    continue;
                }

                let avg_col = Rgb::new(
                    (sum_r / lit) as u8,
                    (sum_g / lit) as u8,
                    (sum_b / lit) as u8,
                );
                let avg_shade = sum_shade / lit as f32;
                let cov = lit as f32 / 8.0;

                let ch = if nose >= 3 {
                    '#'
                } else if eye_glint >= 1 {
                    '*'
                } else if eye_pupil >= 3 {
                    if self.is_blinking { '-' } else { '@' }
                } else if eye_iris >= 3 {
                    '#'
                } else if cov <= 0.25 {
                    '.'
                } else if cov <= 0.45 {
                    ':'
                } else if cov <= 0.68 {
                    '+'
                } else if avg_shade >= 0.88 {
                    '@'
                } else if avg_shade >= 0.74 {
                    '#'
                } else if avg_shade >= 0.64 {
                    '*'
                } else {
                    '+'
                };

                row_cells.push(AsciiCell { ch, fg: avg_col });
            }
            out.push(row_cells);
        }
        out
    }

    /// Renders the 3D Origami Fox into `cols` x `pixel_rows` (`pixel_rows` should be even),
    /// returning `pixel_rows / 2` terminal rows of `(top_rgb, bottom_rgb)` half-block pixels.
    pub fn render_halfblock_grid(
        &self,
        cols: usize,
        pixel_rows: usize,
        bg: Rgb,
    ) -> Vec<Vec<(Rgb, Rgb)>> {
        let w = cols.max(12);
        let h = (pixel_rows.max(8) / 2) * 2;
        let (colors, _, _) = self.rasterize_buffer(w, h, bg, false);

        let term_rows = h / 2;
        let mut out = Vec::with_capacity(term_rows);
        for tr in 0..term_rows {
            let mut row = Vec::with_capacity(w);
            let y_top = tr * 2;
            let y_bot = y_top + 1;
            for x in 0..w {
                row.push((colors[y_top * w + x], colors[y_bot * w + x]));
            }
            out.push(row);
        }
        out
    }
}

fn edge_fn(ax: f32, ay: f32, bx: f32, by: f32, cx: f32, cy: f32) -> f32 {
    (cx - ax) * (by - ay) - (cy - ay) * (bx - ax)
}

fn shade_mat(mat: FacetMat, shade: f32, monochrome: bool) -> Rgb {
    let (r, g, b) = if monochrome {
        match mat {
            FacetMat::Orange => (212.0, 212.0, 212.0),
            FacetMat::OrangeDeep => (148.0, 148.0, 148.0),
            FacetMat::Beige => (246.0, 246.0, 246.0),
            FacetMat::EarInner => (240.0, 240.0, 240.0),
            FacetMat::EarTip => (188.0, 188.0, 188.0),
            FacetMat::Nose => (52.0, 52.0, 52.0),
        }
    } else {
        match mat {
            FacetMat::Orange => (206.0, 118.0, 78.0),
            FacetMat::OrangeDeep => (154.0, 82.0, 54.0),
            FacetMat::Beige => (224.0, 204.0, 172.0),
            FacetMat::EarInner => (232.0, 218.0, 192.0),
            FacetMat::EarTip => (194.0, 102.0, 64.0),
            FacetMat::Nose => (56.0, 58.0, 76.0),
        }
    };
    Rgb::new(
        (r * shade).clamp(0.0, 255.0) as u8,
        (g * shade).clamp(0.0, 255.0) as u8,
        (b * shade).clamp(0.0, 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_non_empty_symmetric_fox_grid() {
        let fox = FoxState::default();
        let bg = Rgb::new(26, 27, 38);
        let grid = fox.render_halfblock_grid(24, 18, bg);
        assert_eq!(grid.len(), 9);
        assert_eq!(grid[0].len(), 24);
        let non_bg: usize = grid
            .iter()
            .flat_map(|r| r.iter())
            .filter(|(t, b)| t.r != bg.r || b.r != bg.r)
            .count();
        assert!(non_bg > 60, "expected solid fox silhouette, got {non_bg}");
    }

    #[test]
    fn renders_cline_style_ascii_fox_grid() {
        let mut fox = FoxState::default();
        let grid_center = fox.render_ascii_grid(36, 12);
        assert_eq!(grid_center.len(), 12);
        assert_eq!(grid_center[0].len(), 36);
        let non_empty = grid_center
            .iter()
            .flat_map(|r| r.iter())
            .filter(|c| c.ch != ' ')
            .count();
        assert!(non_empty > 100);
        for row in &grid_center {
            let s: String = row.iter().map(|c| c.ch).collect();
            println!("{s}");
        }
        fox.yaw = -0.45;
        let grid_left = fox.render_ascii_grid(36, 12);
        println!("---LEFT---");
        for row in &grid_left {
            let s: String = row.iter().map(|c| c.ch).collect();
            println!("{s}");
        }
    }

    #[test]
    fn step_interpolates_yaw_smoothly_toward_target() {
        let mut fox = FoxState::default();
        assert_eq!(fox.pitch, 0.0);
        fox.look_at(-0.8, 0.2);
        let changed = fox.step(0.033, false);
        assert!(changed);
        assert!(fox.yaw < 0.0);
        assert!(fox.yaw > fox.target_yaw);
    }

    #[test]
    fn monochrome_fox_renders_pure_grayscale_cells() {
        let fox = FoxState::default();
        let grid = fox.render_ascii_grid_styled(36, 12, true);
        let non_empty: Vec<_> = grid
            .iter()
            .flat_map(|r| r.iter())
            .filter(|c| c.ch != ' ')
            .collect();
        assert!(non_empty.len() > 80);
        for cell in non_empty {
            assert_eq!(cell.fg.r, cell.fg.g);
            assert_eq!(cell.fg.g, cell.fg.b);
        }
    }

    #[test]
    fn pixel_fox_emblem_is_6_by_16_and_supports_monochrome() {
        let color_grid = render_pixel_fox_emblem(false, false);
        assert_eq!(color_grid.len(), 6);
        assert!(color_grid.iter().all(|r| r.len() == 16));
        let drawn = color_grid
            .iter()
            .flat_map(|r| r.iter())
            .filter(|c| c.ch != ' ')
            .count();
        assert!(drawn > 50);

        let mono_grid = render_pixel_fox_emblem(true, false);
        for cell in mono_grid.iter().flat_map(|r| r.iter()) {
            if let Some(fg) = cell.fg {
                assert_eq!(fg.r, fg.g);
                assert_eq!(fg.g, fg.b);
            }
            if let Some(bg) = cell.bg {
                assert_eq!(bg.r, bg.g);
                assert_eq!(bg.g, bg.b);
            }
        }
    }
}
