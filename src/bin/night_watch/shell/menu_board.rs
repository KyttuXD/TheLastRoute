//! The shift board: nine modes on the left; the selected mode's portrait, text and shift tiles on the right; START SHIFT.
#![allow(dead_code)]

use super::*;

const LIST_X: f32 = 8.0;
const LIST_Y: f32 = 24.0;
const LIST_W: f32 = 120.0;
const ROW_H: f32 = 15.0;
const RIGHT_X: f32 = 134.0;
const RIGHT_W: f32 = 338.0;
const DETAIL_Y: f32 = 24.0;
const DETAIL_H: f32 = 210.0;

impl Shell {
    pub(super) fn select_mode(&mut self, i: usize) {
        if i != self.board_mode {
            self.board_mode = i;
            self.board_level = 0;
            self.info_scroll = 0.0;
        }
    }

    pub(super) fn board(&mut self, c: &mut Ctx<'_>) -> ShellAction {
        let modes = c.modes;
        let mut act = self.header(c, "SHIFT BOARD", "");
        let n = modes.modes.len();
        if n == 0 {
            return act;
        }
        self.board_mode = self.board_mode.min(n - 1);

        // up and down change the mode before anything below reads it
        if c.keys.down {
            self.select_mode(wrap_index(self.board_mode, 1, n));
            c.cue("move");
        }
        if c.keys.up {
            self.select_mode(wrap_index(self.board_mode, -1, n));
            c.cue("move");
        }
        let m = &modes.modes[self.board_mode];
        let levels = m.levels.len();
        self.board_level = self.board_level.min(levels.saturating_sub(1));
        let open: Vec<bool> = (0..levels).map(|i| c.save.level_open(&m.id, i as i32 + 1, m.open_all)).collect();
        let cleared = c.save.cleared_levels(&m.id);

        if c.keys.left || c.keys.right {
            let step = if c.keys.right { 1 } else { -1 };
            let next = next_open_tile(self.board_level, step, &open);
            if next != self.board_level {
                self.board_level = next;
                c.cue("move");
            }
        }
        if c.keys.enter && matches!(act, ShellAction::None) {
            act = self.board_start(c, m);
        }

        // --- mode list
        for (i, mm) in modes.modes.iter().enumerate() {
            let r = Rect::new(LIST_X, LIST_Y + i as f32 * (ROW_H + 1.0), LIST_W, ROW_H);
            let style = if i == self.board_mode { ButtonStyle::Selected } else { ButtonStyle::Row };
            let label = format!("{}.  {}", mm.num, mm.title);
            let res = c.ui.button(r, &fit(&label, LIST_W - 36.0, 6.0), 6.0, style, true);
            let stat = format!("{}/{}", cleared_count(&c.save.cleared_levels(&mm.id), mm.scored), mm.scored);
            text_right(&stat, r.x + r.w - 5.0, r.y + 5.0, 5.0, DIM);
            if res.clicked {
                self.select_mode(i);
                c.cue("pick");
            }
        }

        // --- detail panel
        let detail = Rect::new(RIGHT_X, DETAIL_Y, RIGHT_W, DETAIL_H);
        panel(detail);
        let (ix, iy, iw, ih) = (detail.x + 5.0, detail.y + 5.0, detail.w - 10.0, detail.h - 10.0);

        let cols = tile_columns(levels);
        let rows = (levels + cols - 1) / cols.max(1);
        let tiles_h = |th: f32| if rows == 0 { 0.0 } else { rows as f32 * th + (rows - 1) as f32 * 3.0 };
        let mut th = 24.0;
        let mut top_h = ih - tiles_h(th) - 4.0;
        while top_h < 54.0 && th > 14.0 {
            th -= 2.0;
            top_h = ih - tiles_h(th) - 4.0;
        }

        let art = m.art.first().cloned().unwrap_or_default();
        self.draw_portrait(Rect::new(ix, iy, 96.0, 54.0), if art.is_empty() { &m.label } else { &art });

        // text block: scrolls with the wheel when it is taller than its area
        let info_x = ix + 96.0 + 8.0;
        let info_w = ix + iw - info_x;
        let mut lines: Vec<(String, f32, Color)> = Vec::new();
        lines.push((m.label.clone(), 11.0, INK));
        let block = |lines: &mut Vec<(String, f32, Color)>, s: &str, size: f32, col: Color| {
            if s.is_empty() {
                return;
            }
            lines.push((String::new(), 2.0, col));
            for l in wrap(s, info_w, size) {
                lines.push((l, size, col));
            }
        };
        block(&mut lines, &m.tag, 6.0, AMBER);
        block(&mut lines, &m.blurb, 6.0, INK);
        block(&mut lines, &m.hint, 6.0, DIM);
        let stats = stat_line(
            crate::registry::playable(&m.id),
            cleared_count(&cleared, m.scored),
            m.scored,
            m.presets.len(),
            c.save.star_count(&m.id),
            c.save.best_score(&m.id),
        );
        block(&mut lines, &stats, 5.0, DIM);

        let total: f32 = lines.iter().map(|l| line_height(l.1)).sum();
        let max_scroll = (total - top_h).max(0.0);
        if c.keys.wheel != 0.0 && Rect::new(info_x, iy, info_w, top_h).contains(c.ui.mouse) {
            self.info_scroll -= c.keys.wheel * 10.0;
        }
        self.info_scroll = self.info_scroll.clamp(0.0, max_scroll);
        let mut y = 0.0;
        for (s, size, col) in &lines {
            let lh = line_height(*size);
            let yy = y - self.info_scroll;
            if !s.is_empty() && yy >= 0.0 && yy + lh <= top_h + 0.5 {
                text(s, info_x, iy + yy, *size, *col);
            }
            y += lh;
        }
        if max_scroll > self.info_scroll + 0.5 {
            text_right("v", ix + iw, iy + top_h - 6.0, 5.0, DIM);
        }

        // --- shift tiles
        let tiles_y = iy + top_h + 4.0;
        let tile_w = (iw - 4.0 * (cols - 1) as f32) / cols as f32;
        for i in 0..levels {
            let (col, row) = (i % cols, i / cols);
            let r = Rect::new(ix + col as f32 * (tile_w + 4.0), tiles_y + row as f32 * (th + 3.0), tile_w, th);
            let lines = tile_lines(i, &m.levels[i], open[i], cleared.contains(&(i as u32 + 1)), m.scored);
            let res = c.ui.tile(r, &lines, 6.0, i == self.board_level, open[i]);
            if res.denied {
                c.cue("deny");
            }
            if res.clicked && matches!(act, ShellAction::None) {
                self.board_level = i;
                act = self.board_start(c, m);
            }
        }

        // --- START SHIFT
        let start = Rect::new(RIGHT_X + (RIGHT_W - 148.0) * 0.5, DETAIL_Y + DETAIL_H + 4.0, 148.0, 16.0);
        let res = c.ui.button(start, "▶ START SHIFT  ENTER", 8.0, ButtonStyle::Primary, true);
        if res.clicked && matches!(act, ShellAction::None) {
            act = self.board_start(c, m);
        }
        act
    }

    /// Start the selected shift, or toast why not.
    fn board_start(&mut self, c: &mut Ctx<'_>, m: &ModeEntry) -> ShellAction {
        if !crate::registry::playable(&m.id) {
            self.toast = format!("{} is not built yet. See docs/NIGHT_WATCH_MODES.md.", m.label);
            c.cue("deny");
            return ShellAction::None;
        }
        let level = self.board_level as i32 + 1;
        if !c.save.level_open(&m.id, level, m.open_all) {
            self.toast = "That shift is locked.".to_string();
            c.cue("deny");
            return ShellAction::None;
        }
        ShellAction::StartShift { mode: m.id.clone(), level, custom: BTreeMap::new() }
    }
}
