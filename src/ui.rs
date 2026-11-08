//! Сенсорное управление (Android): плавающий джойстик слева, кнопки
//! атаки/прыжка/рывка справа и кнопка паузы. На десктопе панель скрыта —
//! включается при первом настоящем касании, так что тачскрин-ноутбуки
//! тоже работают. Вся логика раскладки — чистые функции, покрытые тестами.

use macroquad::input::{touches, TouchPhase};
use macroquad::math::Vec2;
use macroquad::prelude::{draw_circle, draw_rectangle, Color};
use macroquad::window::screen_width;
use macroquad::window::screen_height;

/// Радиус основания джойстика, px экрана.
pub const STICK_RADIUS: f32 = 52.0;
/// Мёртвая зона стика — доля радиуса.
const DEAD_ZONE: f32 = 0.16;

/// Центры кнопок (атака, прыжок, рывок) под размер экрана.
/// Чистая функция — покрывается юнит-тестом.
pub fn button_layout(w: f32, h: f32) -> [(f32, f32); 3] {
    [(w - 62.0, h - 62.0), (w - 156.0, h - 44.0), (w - 52.0, h - 150.0)]
}

/// Позиция кнопки паузы (верхний правый угол).
pub fn pause_pos(w: f32) -> (f32, f32) {
    (w - 30.0, 30.0)
}

/// Ось стика по положению пальца: [-1, 1] с мёртвой зоной и clamp.
/// Чистая функция — покрывается юнит-тестом.
pub fn stick_axis(origin: Vec2, knob: Vec2) -> f32 {
    let dx = knob.x - origin.x;
    let len = knob - origin;
    let l = len.length();
    let raw = if l > STICK_RADIUS { dx / l * STICK_RADIUS } else { dx };
    let a = raw / STICK_RADIUS;
    if a.abs() < DEAD_ZONE {
        0.0
    } else {
        a.clamp(-1.0, 1.0)
    }
}

fn inside(p: Vec2, c: (f32, f32), r: f32) -> bool {
    (p - Vec2::new(c.0, c.1)).length() <= r
}

/// Состояние одной кнопки: каким пальцем зажата.
#[derive(Default)]
struct Button {
    finger: Option<u64>,
    held: bool,
    edge: bool,
}

/// Сенсорная панель управления. Обновляется раз в кадр до фиксированных
/// шагов; edge-нажатия потребляет [`Game`](crate::game).
pub struct TouchUi {
    /// Панель видима и слушает касания.
    pub enabled: bool,
    /// Палец джойстика.
    stick_finger: Option<u64>,
    stick_origin: Vec2,
    stick_knob: Vec2,
    axis: f32,
    jump: Button,
    dash: Button,
    atk: Button,
    pause_edge: bool,
}

impl TouchUi {
    pub fn new() -> Self {
        TouchUi {
            enabled: false,
            stick_finger: None,
            stick_origin: Vec2::ZERO,
            stick_knob: Vec2::ZERO,
            axis: 0.0,
            jump: Button::default(),
            dash: Button::default(),
            atk: Button::default(),
            pause_edge: false,
        }
    }

    /// Опрос касаний (каждый кадр отрисовки).
    pub fn update(&mut self) {
        let w = screen_width();
        let h = screen_height();
        let [atk_b, jump_b, dash_b] = button_layout(w, h);
        let pause_b = pause_pos(w);

        let mut events = touches();
        if !events.is_empty() {
            self.enabled = true;
        }

        for t in events.drain(..) {
            let p = t.position;
            match t.phase {
                TouchPhase::Started => {
                    if inside(p, pause_b, 26.0) {
                        self.pause_edge = true;
                    } else if p.x < w * 0.45 && self.stick_finger.is_none() {
                        // Джойстик появляется там, где легал палец.
                        self.stick_finger = Some(t.id);
                        self.stick_origin = p;
                        self.stick_knob = p;
                        self.axis = 0.0;
                    } else if inside(p, jump_b, 42.0) && self.jump.finger.is_none() {
                        self.jump.finger = Some(t.id);
                        self.jump.held = true;
                        self.jump.edge = true;
                    } else if inside(p, dash_b, 36.0) && self.dash.finger.is_none() {
                        self.dash.finger = Some(t.id);
                        self.dash.held = true;
                        self.dash.edge = true;
                    } else if inside(p, atk_b, 50.0) && self.atk.finger.is_none() {
                        self.atk.finger = Some(t.id);
                        self.atk.held = true;
                        self.atk.edge = true;
                    }
                }
                TouchPhase::Moved | TouchPhase::Stationary => {
                    if self.stick_finger == Some(t.id) {
                        self.stick_knob = p;
                        self.axis = stick_axis(self.stick_origin, p);
                        // Плавающее основание: подтягивается за пальцем.
                        let d = p - self.stick_origin;
                        let l = d.length();
                        if l > STICK_RADIUS {
                            self.stick_origin += d / l * (l - STICK_RADIUS);
                        }
                    } else if self.jump.finger == Some(t.id) && !inside(p, jump_b, 76.0) {
                        self.jump.finger = None;
                        self.jump.held = false;
                    } else if self.dash.finger == Some(t.id) && !inside(p, dash_b, 65.0) {
                        self.dash.finger = None;
                        self.dash.held = false;
                    } else if self.atk.finger == Some(t.id) && !inside(p, atk_b, 90.0) {
                        self.atk.finger = None;
                        self.atk.held = false;
                    }
                }
                TouchPhase::Ended | TouchPhase::Cancelled => {
                    if self.stick_finger == Some(t.id) {
                        self.stick_finger = None;
                        self.stick_knob = self.stick_origin;
                        self.axis = 0.0;
                    }
                    for b in [&mut self.jump, &mut self.dash, &mut self.atk] {
                        if b.finger == Some(t.id) {
                            b.finger = None;
                            b.held = false;
                        }
                    }
                }
            }
        }
    }

    /// Ось движения от стика: -1..1 (0, если панель скрыта).
    pub fn axis_x(&self) -> f32 {
        if self.enabled {
            self.axis
        } else {
            0.0
        }
    }

    /// Кнопка прыжка удерживается.
    pub fn jump_held(&self) -> bool {
        self.enabled && self.jump.held
    }

    pub fn take_jump_edge(&mut self) -> bool {
        std::mem::take(&mut self.jump.edge)
    }

    pub fn take_dash_edge(&mut self) -> bool {
        std::mem::take(&mut self.dash.edge)
    }

    pub fn take_attack_edge(&mut self) -> bool {
        std::mem::take(&mut self.atk.edge)
    }

    /// Нажата кнопка паузы (одноразово).
    pub fn take_pause(&mut self) -> bool {
        std::mem::take(&mut self.pause_edge)
    }

    /// Сбросить накопленные edge (при паузе, чтобы не «выстреливали» потом).
    pub fn clear_edges(&mut self) {
        self.jump.edge = false;
        self.dash.edge = false;
        self.atk.edge = false;
    }

    /// Отрисовка панели поверх HUD (screen-space).
    pub fn draw(&self) {
        if !self.enabled {
            return;
        }
        let w = screen_width();
        let h = screen_height();
        let [atk_b, jump_b, dash_b] = button_layout(w, h);
        let (px, py) = pause_pos(w);

        // --- джойстик: основание, кольцо, ручка ---
        if self.stick_finger.is_some() {
            draw_circle(
                self.stick_origin.x,
                self.stick_origin.y,
                STICK_RADIUS + 3.0,
                Color::new(0.05, 0.04, 0.035, 0.30),
            );
            draw_circle(
                self.stick_origin.x,
                self.stick_origin.y,
                STICK_RADIUS,
                Color::new(0.85, 0.82, 0.74, 0.07),
            );
            draw_circle(
                self.stick_origin.x,
                self.stick_origin.y,
                STICK_RADIUS - 4.0,
                Color::new(0.05, 0.04, 0.035, 0.22),
            );
            draw_circle(self.stick_knob.x, self.stick_knob.y, 24.0, Color::new(0.05, 0.04, 0.035, 0.45));
            draw_circle(self.stick_knob.x, self.stick_knob.y, 20.0, Color::new(0.85, 0.82, 0.74, 0.30));
            draw_circle(
                self.stick_knob.x - 3.0,
                self.stick_knob.y - 4.0,
                7.0,
                Color::new(0.95, 0.93, 0.86, 0.35),
            );
        }

        // --- кнопки: диск + иконка ---
        self.draw_button(atk_b, 40.0, self.atk.held);
        self.draw_sword_icon(atk_b.0, atk_b.1);
        self.draw_button(jump_b, 34.0, self.jump.held);
        self.draw_arrow_icon(jump_b.0, jump_b.1, -1.0);
        self.draw_button(dash_b, 28.0, self.dash.held);
        self.draw_arrow_icon(dash_b.0, dash_b.1, 1.0);

        // --- пауза: две вертикальные полосы ---
        draw_circle(px, py, 22.0, Color::new(0.05, 0.04, 0.035, 0.35));
        draw_circle(px, py, 19.0, Color::new(0.30, 0.27, 0.24, 0.55));
        draw_rectangle(px - 6.0, py - 7.0, 4.0, 14.0, Color::new(0.90, 0.87, 0.80, 0.9));
        draw_rectangle(px + 2.0, py - 7.0, 4.0, 14.0, Color::new(0.90, 0.87, 0.80, 0.9));
    }

    fn draw_button(&self, c: (f32, f32), r: f32, pressed: bool) {
        let a_fill = if pressed { 0.34 } else { 0.16 };
        let a_ring = if pressed { 0.85 } else { 0.45 };
        draw_circle(c.0, c.1, r + 3.0, Color::new(0.05, 0.04, 0.035, 0.40));
        draw_circle(c.0, c.1, r, Color::new(0.85, 0.82, 0.74, a_fill));
        draw_circle(c.0, c.1, r - 2.5, Color::new(0.05, 0.04, 0.035, 0.30));
        // Тонкое световое кольцо (из точек-секторов по окружности).
        let ring = 14;
        for i in 0..ring {
            let a = i as f32 / ring as f32 * std::f32::consts::TAU;
            let sxp = c.0 + a.cos() * (r - 0.5);
            let syp = c.1 + a.sin() * (r - 0.5);
            draw_rectangle(
                sxp - 1.0,
                syp - 1.0,
                2.0,
                2.0,
                Color::new(0.85, 0.82, 0.74, a_ring * 0.7),
            );
        }
    }

    /// Иконка меча (кнопка атаки).
    fn draw_sword_icon(&self, cx: f32, cy: f32) {
        let steel = Color::new(0.90, 0.88, 0.82, 0.95);
        let dark = Color::new(0.15, 0.13, 0.11, 0.95);
        let gold = Color::new(0.769, 0.627, 0.290, 0.95);
        // Клинок.
        draw_rectangle(cx - 2.0, cy - 20.0, 4.0, 22.0, steel);
        // Острие.
        draw_rectangle(cx - 1.0, cy - 23.0, 2.0, 3.0, steel);
        // Гарда.
        draw_rectangle(cx - 8.0, cy + 1.0, 16.0, 3.0, gold);
        // Рукоять.
        draw_rectangle(cx - 1.5, cy + 4.0, 3.0, 8.0, dark);
        // Навершие.
        draw_rectangle(cx - 3.0, cy + 12.0, 6.0, 3.0, gold);
    }

    /// Иконка-шеврон: вверх (dir = -1) для прыжка, вправо (dir = 1) для рывка.
    fn draw_arrow_icon(&self, cx: f32, cy: f32, dir: f32) {
        let c = Color::new(0.92, 0.90, 0.84, 0.95);
        if dir < 0.0 {
            // Вверх: ступенчатая стрелка.
            draw_rectangle(cx - 10.0, cy + 3.0, 20.0, 3.5, c);
            draw_rectangle(cx - 7.0, cy - 1.0, 14.0, 4.0, c);
            draw_rectangle(cx - 4.0, cy - 5.0, 8.0, 4.0, c);
            draw_rectangle(cx - 1.5, cy - 9.0, 3.0, 4.0, c);
        } else {
            // Вправо: двойной шеврон рывка.
            for k in 0..2 {
                let ox = cx - 9.0 + k as f32 * 9.0;
                draw_rectangle(ox, cy - 6.0, 3.5, 12.0, c);
                draw_rectangle(ox + 3.5, cy - 3.0, 3.0, 6.0, c);
                draw_rectangle(ox + 6.5, cy - 1.0, 2.5, 2.0, c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_fits_landscape_screen() {
        let w = 800.0;
        let h = 360.0;
        let [atk, jump, dash] = button_layout(w, h);
        let (px, py) = pause_pos(w);
        for (x, y) in [atk, jump, dash, (px, py)] {
            assert!(x > 0.0 && x < w, "кнопка вне экрана по X: {x}");
            assert!(y > 0.0 && y < h, "кнопка вне экрана по Y: {y}");
        }
        // Атака — правый нижний угол, джойстик не мешает (левая половина).
        assert!(atk.0 > w * 0.5);
        assert!((px - w).abs() < 60.0, "пауза в правом верхнем углу");
    }

    #[test]
    fn stick_axis_clamps_and_deadzones() {
        let o = Vec2::new(100.0, 100.0);
        assert_eq!(stick_axis(o, o), 0.0, "палец в центре — ноль");
        assert_eq!(stick_axis(o, Vec2::new(100.0 + 4.0, 100.0)), 0.0, "мёртвая зона");
        let full = stick_axis(o, Vec2::new(100.0 + STICK_RADIUS * 2.0, 100.0));
        assert!((full - 1.0).abs() < 0.001, "clamp дальнего пальца к 1.0");
        let half = stick_axis(o, Vec2::new(100.0 + STICK_RADIUS * 0.5, 100.0));
        assert!((half - 0.5).abs() < 0.001, "полоборота = 0.5");
        let neg = stick_axis(o, Vec2::new(100.0 - STICK_RADIUS, 100.0));
        assert!((neg + 1.0).abs() < 0.001);
        // Наклонный палец за радиусом не даёт значения больше 1.
        let diag = stick_axis(o, Vec2::new(100.0 + 300.0, 100.0 - 300.0));
        assert!(diag.abs() <= 1.0);
    }
}
