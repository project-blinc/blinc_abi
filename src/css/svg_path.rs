//! SVG path data, the language of a `<path>`'s `d`, read and cut into rings of
//! straight steps: `M 4 12 l 5 5 L 20 6` and its compact forms such as
//! `M4,12l5,5L20,6` or `M.5-1.5.5.5`.

/// One step of an outline, in absolute coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Move(f64, f64),
    Line(f64, f64),
    /// Two control points, then the end.
    Cubic(f64, f64, f64, f64, f64, f64),
    /// One control point, then the end.
    Quad(f64, f64, f64, f64),
    Arc {
        rx: f64,
        ry: f64,
        /// Degrees the ellipse's x axis is turned by.
        rotation: f64,
        large: bool,
        sweep: bool,
        x: f64,
        y: f64,
    },
    Close,
}

/// The commands of `d`, relative steps, `H`, `V` and smooth curves made absolute; an error
/// saying where the first mistake is.
pub fn parse(d: &str) -> Result<Vec<Command>, String> {
    Reader {
        d: d.as_bytes(),
        i: 0,
        x: 0.0,
        y: 0.0,
        start: (0.0, 0.0),
        last_cubic: None,
        last_quad: None,
        out: Vec::new(),
    }
    .read()
}

/// The subpaths of `commands` as closed rings of points, the last the first again; curves and
/// arcs are cut into straight steps of about `step` units, at most 256 to one.
pub fn flatten(commands: &[Command], step: f64) -> Vec<Vec<(f64, f64)>> {
    struct Walk {
        rings: Vec<Vec<(f64, f64)>>,
        ring: Vec<(f64, f64)>,
        at: (f64, f64),
    }
    impl Walk {
        fn close(&mut self) {
            let mut ring = std::mem::take(&mut self.ring);
            if ring.len() >= 3 {
                if ring.first() != ring.last() {
                    ring.push(ring[0]);
                }
                self.rings.push(ring);
            }
        }
        fn to(&mut self, p: (f64, f64)) {
            if self.ring.is_empty() {
                self.ring.push(self.at);
            }
            self.ring.push(p);
            self.at = p;
        }
    }
    let steps = |length: f64| (length / step).ceil().clamp(1.0, 256.0) as usize;
    let dist = |a: (f64, f64), b: (f64, f64)| ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let mut w = Walk {
        rings: Vec::new(),
        ring: Vec::new(),
        at: (0.0, 0.0),
    };
    let mut start = (0.0, 0.0);
    for &c in commands {
        match c {
            Command::Move(x, y) => {
                w.close();
                w.at = (x, y);
                start = (x, y);
            }
            Command::Line(x, y) => w.to((x, y)),
            Command::Quad(x1, y1, x, y) => {
                let (p0, p1, p2) = (w.at, (x1, y1), (x, y));
                let n = steps(dist(p0, p1) + dist(p1, p2));
                for k in 1..=n {
                    let t = k as f64 / n as f64;
                    let u = 1.0 - t;
                    w.to((
                        u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0,
                        u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1,
                    ));
                }
            }
            Command::Cubic(x1, y1, x2, y2, x, y) => {
                let (p0, p1, p2, p3) = (w.at, (x1, y1), (x2, y2), (x, y));
                let n = steps(dist(p0, p1) + dist(p1, p2) + dist(p2, p3));
                for k in 1..=n {
                    let t = k as f64 / n as f64;
                    let u = 1.0 - t;
                    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    w.to((
                        a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
                        a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
                    ));
                }
            }
            Command::Arc {
                rx,
                ry,
                rotation,
                large,
                sweep,
                x,
                y,
            } => {
                let from = w.at;
                arc(
                    from,
                    (rx, ry),
                    rotation,
                    (large, sweep),
                    (x, y),
                    step,
                    &mut |p| w.to(p),
                );
            }
            Command::Close => {
                w.to(start);
                w.close();
                w.at = start;
            }
        }
    }
    w.close();
    w.rings
}

/// Calls `to` along an arc from `from` to `end` by its centre form (SVG 1.1, F.6.5).
fn arc(
    (x1, y1): (f64, f64),
    (rx, ry): (f64, f64),
    rotation: f64,
    (large, sweep): (bool, bool),
    (x2, y2): (f64, f64),
    step: f64,
    to: &mut dyn FnMut((f64, f64)),
) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 || (x1 == x2 && y1 == y2) {
        to((x2, y2));
        return;
    }
    let phi = rotation.to_radians();
    let (cos, sin) = (phi.cos(), phi.sin());
    let (dx, dy) = ((x1 - x2) / 2.0, (y1 - y2) / 2.0);
    let (px, py) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    // Radii too small to reach are scaled up until they just do.
    let lambda = px * px / (rx * rx) + py * py / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * py * py - ry * ry * px * px;
    let den = rx * rx * py * py + ry * ry * px * px;
    let coef = if large == sweep { -1.0 } else { 1.0 } * (num / den).max(0.0).sqrt();
    let (cpx, cpy) = (coef * rx * py / ry, -coef * ry * px / rx);
    let cx = cos * cpx - sin * cpy + (x1 + x2) / 2.0;
    let cy = sin * cpx + cos * cpy + (y1 + y2) / 2.0;
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let start = angle(1.0, 0.0, (px - cpx) / rx, (py - cpy) / ry);
    let mut span = angle(
        (px - cpx) / rx,
        (py - cpy) / ry,
        (-px - cpx) / rx,
        (-py - cpy) / ry,
    );
    if !sweep && span > 0.0 {
        span -= std::f64::consts::TAU;
    } else if sweep && span < 0.0 {
        span += std::f64::consts::TAU;
    }
    let n = (span.abs() * rx.max(ry) / step).ceil().clamp(1.0, 256.0) as usize;
    for k in 1..=n {
        let t = start + span * k as f64 / n as f64;
        to((
            cos * rx * t.cos() - sin * ry * t.sin() + cx,
            sin * rx * t.cos() + cos * ry * t.sin() + cy,
        ));
    }
}

struct Reader<'a> {
    d: &'a [u8],
    i: usize,
    x: f64,
    y: f64,
    start: (f64, f64),
    /// The last control point of a cubic or a quadratic curve, which a smooth one reflects.
    last_cubic: Option<(f64, f64)>,
    last_quad: Option<(f64, f64)>,
    out: Vec<Command>,
}

fn is_command(c: u8) -> bool {
    b"MmLlHhVvCcSsQqTtAaZz".contains(&c)
}

impl Reader<'_> {
    fn fail<T>(&self, message: impl std::fmt::Display) -> Result<T, String> {
        Err(format!("path data, at {}: {message}", self.i))
    }

    fn read(mut self) -> Result<Vec<Command>, String> {
        self.skip();
        if self.i < self.d.len() && !matches!(self.d[self.i], b'M' | b'm') {
            return self.fail("a path starts with M");
        }
        let mut command = 0u8;
        loop {
            self.skip();
            let Some(&c) = self.d.get(self.i) else {
                break;
            };
            if is_command(c) {
                command = c;
                self.i += 1;
            } else if command == 0 || command == b'Z' || command == b'z' {
                return self.fail(format!("expected a command, not \"{}\"", c as char));
            }
            self.step(command)?;
            // After a move, further pairs are lines.
            command = match command {
                b'M' => b'L',
                b'm' => b'l',
                other => other,
            };
        }
        Ok(self.out)
    }

    fn step(&mut self, command: u8) -> Result<(), String> {
        let relative = command.is_ascii_lowercase();
        let (ox, oy) = if relative {
            (self.x, self.y)
        } else {
            (0.0, 0.0)
        };
        let (mut cubic, mut quad) = (None, None);
        match command.to_ascii_uppercase() {
            b'M' => {
                self.x = ox + self.number()?;
                self.y = oy + self.number()?;
                self.start = (self.x, self.y);
                self.out.push(Command::Move(self.x, self.y));
            }
            b'L' => {
                self.x = ox + self.number()?;
                self.y = oy + self.number()?;
                self.out.push(Command::Line(self.x, self.y));
            }
            b'H' => {
                self.x = ox + self.number()?;
                self.out.push(Command::Line(self.x, self.y));
            }
            b'V' => {
                self.y = oy + self.number()?;
                self.out.push(Command::Line(self.x, self.y));
            }
            b'C' => {
                let (x1, y1) = (ox + self.number()?, oy + self.number()?);
                let (x2, y2) = (ox + self.number()?, oy + self.number()?);
                self.x = ox + self.number()?;
                self.y = oy + self.number()?;
                self.out
                    .push(Command::Cubic(x1, y1, x2, y2, self.x, self.y));
                cubic = Some((x2, y2));
            }
            b'S' => {
                let (x1, y1) = match self.last_cubic {
                    Some((cx, cy)) => (2.0 * self.x - cx, 2.0 * self.y - cy),
                    None => (self.x, self.y),
                };
                let (x2, y2) = (ox + self.number()?, oy + self.number()?);
                self.x = ox + self.number()?;
                self.y = oy + self.number()?;
                self.out
                    .push(Command::Cubic(x1, y1, x2, y2, self.x, self.y));
                cubic = Some((x2, y2));
            }
            b'Q' => {
                let (x1, y1) = (ox + self.number()?, oy + self.number()?);
                self.x = ox + self.number()?;
                self.y = oy + self.number()?;
                self.out.push(Command::Quad(x1, y1, self.x, self.y));
                quad = Some((x1, y1));
            }
            b'T' => {
                let (x1, y1) = match self.last_quad {
                    Some((cx, cy)) => (2.0 * self.x - cx, 2.0 * self.y - cy),
                    None => (self.x, self.y),
                };
                self.x = ox + self.number()?;
                self.y = oy + self.number()?;
                self.out.push(Command::Quad(x1, y1, self.x, self.y));
                quad = Some((x1, y1));
            }
            b'A' => {
                let (rx, ry) = (self.number()?.abs(), self.number()?.abs());
                let rotation = self.number()?;
                let (large, sweep) = (self.flag()?, self.flag()?);
                self.x = ox + self.number()?;
                self.y = oy + self.number()?;
                self.out.push(Command::Arc {
                    rx,
                    ry,
                    rotation,
                    large,
                    sweep,
                    x: self.x,
                    y: self.y,
                });
            }
            _ => {
                (self.x, self.y) = self.start;
                self.out.push(Command::Close);
            }
        }
        self.last_cubic = cubic;
        self.last_quad = quad;
        Ok(())
    }

    /// Whitespace and at most one comma.
    fn skip(&mut self) {
        let mut comma = false;
        while let Some(&c) = self.d.get(self.i) {
            match c {
                b' ' | b'\t' | b'\n' | b'\r' | 0x0C => {}
                b',' if !comma => comma = true,
                _ => break,
            }
            self.i += 1;
        }
    }

    /// An arc flag, a lone 0 or 1, which may run straight into the next number.
    fn flag(&mut self) -> Result<bool, String> {
        self.skip();
        match self.d.get(self.i) {
            Some(&c @ (b'0' | b'1')) => {
                self.i += 1;
                Ok(c == b'1')
            }
            _ => self.fail("an arc flag is 0 or 1"),
        }
    }

    fn number(&mut self) -> Result<f64, String> {
        self.skip();
        let start = self.i;
        let digits = |r: &mut Self| {
            let from = r.i;
            while r.d.get(r.i).is_some_and(u8::is_ascii_digit) {
                r.i += 1;
            }
            r.i - from
        };
        if matches!(self.d.get(self.i), Some(b'+' | b'-')) {
            self.i += 1;
        }
        let mut count = digits(self);
        if self.d.get(self.i) == Some(&b'.') {
            self.i += 1;
            count += digits(self);
        }
        if count == 0 {
            return if self.i >= self.d.len() {
                self.fail("the path ends in the middle of a command")
            } else {
                let near = String::from_utf8_lossy(&self.d[start..(start + 8).min(self.d.len())]);
                self.fail(format!("expected a number at \"{near}\""))
            };
        }
        if matches!(self.d.get(self.i), Some(b'e' | b'E')) {
            let mark = self.i;
            self.i += 1;
            if matches!(self.d.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if digits(self) == 0 {
                self.i = mark;
            }
        }
        // The slice is ASCII digits, signs, a point and an exponent.
        std::str::from_utf8(&self.d[start..self.i])
            .ok()
            .and_then(|s| s.parse().ok())
            .map_or_else(|| self.fail("a number out of range"), Ok)
    }
}
