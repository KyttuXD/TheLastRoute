//! Custom Night: a tab per mode that has one, a slider per hunter, presets, All 0/10/20/Random, the score line, START NIGHT.
#![allow(dead_code)]

use super::*;

#[derive(Clone, Copy)]
enum Quick {
    Preset(usize),
    All(i32),
    Random,
}

impl Shell {
    pub(super) fn custom(&mut self, c: &mut Ctx<'_>) -> ShellAction {
        let modes = c.modes;
        let all = custom_modes(modes);
        let open: Vec<&ModeEntry> = all.iter().copied().filter(|m| c.save.custom_unlocked(&m.id)).collect();
        if open.is_empty() {
            let act = self.header(c, "CUSTOM NIGHT", "");
            text("CLEAR NIGHT 5 OF A MODE TO OPEN ITS CUSTOM NIGHT.", 12.0, 34.0, 7.0, BODY);
            return act;
        }
        let m: &ModeEntry = open.iter().copied().find(|m| m.id == self.custom_mode).unwrap_or(open[0]);
        if self.custom_loaded != m.id {
            self.custom_mode = m.id.clone();
            self.custom_loaded = m.id.clone();
            self.custom_vals = custom_default(m, &c.save.custom_values(&m.id));
            self.custom_dirty = false;
        }
        let extra = format!("{}/{} ★", c.save.star_count(&m.id), m.presets.len());
        let mut act = self.header(c, "CUSTOM NIGHT", &extra);

        // --- tabs
        let labels: Vec<String> = all
            .iter()
            .map(|cm| if c.save.custom_unlocked(&cm.id) { cm.label.clone() } else { format!("{} 🔒", cm.label) })
            .collect();
        let widths: Vec<f32> = labels.iter().map(|l| measure(l, 5.0) + 10.0).collect();
        let (tab_rects, tabs_bottom) = flow_rects(&widths, 8.0, 26.0, 472.0, 12.0, 3.0, 2.0);
        for (i, cm) in all.iter().enumerate() {
            let unlocked = c.save.custom_unlocked(&cm.id);
            let style = if cm.id == m.id { ButtonStyle::Selected } else { ButtonStyle::Tab };
            let res = c.ui.button(tab_rects[i], &labels[i], 5.0, style, unlocked);
            if res.denied {
                c.cue("deny");
            }
            if res.clicked && cm.id != m.id {
                self.custom_mode = cm.id.clone();
                self.custom_loaded.clear();
                c.cue("pick");
            }
        }

        // --- sliders
        let slots = custom_slots(m);
        let sliders_y = (tabs_bottom + 4.0).max(42.0);
        for (i, (id, label)) in slots.iter().enumerate() {
            let y = sliders_y + i as f32 * 12.0;
            text(label, 8.0, y + 2.0, 7.0, INK);
            let cur = self.custom_vals.get(id).copied().unwrap_or(0);
            let nv = c.ui.slider_step(Rect::new(84.0, y + 1.0, 150.0, 10.0), cur as f32, 0.0, MAX_AI as f32, 1.0).round() as i32;
            if nv != cur {
                self.custom_vals.insert(id.clone(), nv);
                self.custom_dirty = true;
            }
            text(&nv.to_string(), 240.0, y + 2.0, 7.0, INK);
        }
        if self.custom_dirty {
            c.save.set_custom_values(&m.id, self.custom_vals.clone());
            if !c.input.click_down {
                c.save.store();
                self.custom_dirty = false;
            }
        }

        // --- presets and quick fills
        let stars: Vec<String> = c.save.stars.get(&m.id).cloned().unwrap_or_default();
        let mut buttons: Vec<(String, Quick)> = Vec::new();
        for (i, p) in m.presets.iter().enumerate() {
            let star = if stars.contains(&p.id) { "★" } else { "☆" };
            buttons.push((format!("{} {}", star, p.name), Quick::Preset(i)));
        }
        buttons.push(("All 0".to_string(), Quick::All(0)));
        buttons.push(("All 10".to_string(), Quick::All(10)));
        buttons.push(("All 20".to_string(), Quick::All(20)));
        buttons.push(("Random".to_string(), Quick::Random));
        let bw: Vec<f32> = buttons.iter().map(|(l, _)| measure(l, 6.0) + 10.0).collect();
        let presets_y = sliders_y + slots.len() as f32 * 12.0 + 8.0;
        let (brects, presets_bottom) = flow_rects(&bw, 8.0, presets_y, 472.0, 14.0, 3.0, 2.0);
        for (i, (label, q)) in buttons.iter().enumerate() {
            let res = c.ui.button(brects[i], label, 6.0, ButtonStyle::Normal, true);
            if res.clicked {
                let v = match *q {
                    Quick::Preset(k) => preset_values(m, &m.presets[k]),
                    Quick::All(n) => all_values(m, n),
                    Quick::Random => random_values(m, &mut self.rng),
                };
                self.custom_vals = v.clone();
                c.save.set_custom_values(&m.id, v);
                c.save.store();
                self.custom_dirty = false;
                c.cue("pick");
            }
        }

        // --- score and start
        let score = custom_score(&self.custom_vals);
        let line = score_line(score, custom_max(slots.len()), c.save.best_score(&m.id));
        text(&line, 8.0, presets_bottom + 6.0, 7.0, INK);
        let go = c.ui.button(Rect::new(190.0, 224.0, 100.0, 16.0), "▶ START NIGHT", 8.0, ButtonStyle::Primary, true);
        if go.clicked && matches!(act, ShellAction::None) {
            act = self.custom_start(c, m);
        }
        act
    }

    fn custom_start(&mut self, c: &mut Ctx<'_>, m: &ModeEntry) -> ShellAction {
        if custom_score(&self.custom_vals) == 0 {
            self.toast = "Set at least one hunter above 0.".to_string();
            c.cue("deny");
            return ShellAction::None;
        }
        if !crate::registry::playable(&m.id) {
            self.toast = format!("{} is not built yet. See docs/NIGHT_WATCH_MODES.md.", m.label);
            c.cue("deny");
            return ShellAction::None;
        }
        c.save.set_custom_values(&m.id, self.custom_vals.clone());
        c.save.store();
        ShellAction::StartShift { mode: m.id.clone(), level: 1, custom: self.custom_vals.clone() }
    }
}
