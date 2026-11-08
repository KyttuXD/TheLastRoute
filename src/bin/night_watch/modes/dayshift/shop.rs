//! Salvage Run's day: the shop (`dayshift_shop.gd`). A panel with the week's budget, tonight's quota and the five upgrades as
//! toggle buttons, and START SHIFT. `sim::shop_toggle` decides both the greyed-out buttons and the click, so the two cannot
//! disagree. `ShopState` is pure (and tested); `draw_shop` draws it with the shared UI kit on the 480x270 canvas.
#![allow(dead_code)]

use super::sim::{budget_for, quota_for, shop_toggle, total_cost, tun};
use crate::core::InputFrame;
use crate::ui::theme::{panel, to_virtual, AMBER, BODY, DIM, INK};
use crate::ui::{text, text_center, ButtonStyle, Ui};
use macroquad::prelude::{draw_rectangle, screen_height, screen_width, Color, Rect};

/// One upgrade button as the shop shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct ShopRow {
    pub id: String,
    pub text: String,
    pub has: bool,
    /// Greyed out: buying it would go over the budget.
    pub disabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShopAction {
    None,
    /// START SHIFT (the basket is `ShopState::owned`).
    Start,
    Toggled { id: String, on: bool },
    /// A click on a greyed-out upgrade.
    Denied,
}

#[derive(Clone, Debug)]
pub struct ShopState {
    /// The week shown (1..=5).
    pub week: i32,
    /// Last week's surplus, when playing on.
    pub carry: i32,
    pub custom: bool,
    pub owned: Vec<String>,
}

impl ShopState {
    pub fn new(week: i32, carry: i32, custom: bool) -> ShopState {
        ShopState { week: week.clamp(1, 5), carry: carry.max(0), custom, owned: Vec::new() }
    }

    /// The week the budget and quota tables use (a Custom Night uses week 4's).
    pub fn table_week(&self) -> i32 {
        if self.custom {
            4
        } else {
            self.week
        }
    }

    pub fn quota(&self) -> i32 {
        quota_for(self.table_week())
    }

    pub fn budget(&self) -> i32 {
        budget_for(self.table_week(), self.carry)
    }

    /// What is left to spend.
    pub fn left(&self) -> i32 {
        self.budget() - total_cost(&self.owned)
    }

    /// Click on an upgrade: false when it would go over the budget (the basket is unchanged).
    pub fn toggle(&mut self, id: &str) -> bool {
        match shop_toggle(&self.owned, id, self.table_week(), self.carry) {
            Some(next) => {
                self.owned = next;
                true
            }
            None => false,
        }
    }

    pub fn has(&self, id: &str) -> bool {
        self.owned.iter().any(|o| o == id)
    }

    /// "DAY · WEEK 3" / "DAY · CUSTOM".
    pub fn kicker(&self) -> String {
        format!("DAY \u{b7} {}", if self.custom { "CUSTOM".to_string() } else { format!("WEEK {}", self.week) })
    }

    pub fn budget_line(&self) -> String {
        let carried = self.budget() - budget_for(self.table_week(), 0);
        let extra = if carried > 0 { format!(" (incl. ${carried} carried over)") } else { String::new() };
        format!("Budget ${} of ${}{}", self.left(), self.budget(), extra)
    }

    pub fn quota_line(&self) -> String {
        format!("Tonight's quota ${}", self.quota())
    }

    pub fn rows(&self) -> Vec<ShopRow> {
        tun()
            .upgrades
            .iter()
            .map(|u| {
                let has = self.has(&u.id);
                let disabled = !has && shop_toggle(&self.owned, &u.id, self.table_week(), self.carry).is_none();
                ShopRow {
                    id: u.id.clone(),
                    text: format!("{} {}  ${}  {}", if has { "\u{2714}" } else { " " }, u.name.to_uppercase(), u.cost, u.blurb),
                    has,
                    disabled,
                }
            })
            .collect()
    }
}

/// Layout in the 480x270 canvas.
const PANEL: Rect = Rect { x: 66.0, y: 12.0, w: 348.0, h: 250.0 };
const ROW_X: f32 = 74.0;
const ROW_W: f32 = 332.0;
const ROW_Y: f32 = 96.0;
const ROW_H: f32 = 20.0;
const ROW_STEP: f32 = 24.0;

pub fn row_rect(i: usize) -> Rect {
    Rect::new(ROW_X, ROW_Y + ROW_STEP * i as f32, ROW_W, ROW_H)
}

pub fn start_rect() -> Rect {
    Rect::new(ROW_X, ROW_Y + ROW_STEP * 5.0 + 6.0, ROW_W, 24.0)
}

/// Draw the shop over the world and read its clicks. `paused`: nothing is clickable (the pause card is up).
pub fn draw_shop(state: &mut ShopState, input: &InputFrame, paused: bool) -> ShopAction {
    // the shop has the screen: cover the Hud and the world behind it
    draw_rectangle(0.0, 0.0, screen_width(), screen_height(), Color::new(0.02, 0.02, 0.04, 0.985));
    panel(PANEL);
    text(&state.kicker(), ROW_X, 20.0, 7.0, AMBER);
    text("THE SHOP", ROW_X, 32.0, 18.0, INK);
    text(&state.budget_line(), ROW_X, 64.0, 8.0, INK);
    text(&state.quota_line(), ROW_X, 76.0, 8.0, BODY);
    let mut ui = Ui::at(to_virtual(input.mouse), input.click && !paused, input.click_down);
    let mut action = ShopAction::None;
    let rows = state.rows();
    for (i, row) in rows.iter().enumerate() {
        let style = if row.has { ButtonStyle::Selected } else { ButtonStyle::Row };
        let res = ui.button(row_rect(i), &row.text, 6.0, style, !row.disabled);
        if res.clicked {
            let on = !row.has;
            if state.toggle(&row.id) {
                action = ShopAction::Toggled { id: row.id.clone(), on };
            }
        } else if res.denied {
            action = ShopAction::Denied;
        }
    }
    if !paused {
        if let Some(n) = input.digit {
            if (1..=5).contains(&n) {
                if let Some(row) = rows.get(n as usize - 1) {
                    if !row.disabled || row.has {
                        let on = !row.has;
                        if state.toggle(&row.id) {
                            action = ShopAction::Toggled { id: row.id.clone(), on };
                        }
                    } else {
                        action = ShopAction::Denied;
                    }
                }
            }
        }
    }
    let go = ui.button(start_rect(), "\u{25b6} START SHIFT", 8.0, ButtonStyle::Primary, true);
    if go.clicked || (!paused && input.just("interact")) {
        action = ShopAction::Start;
    }
    text_center("1-5 toggle an upgrade     E or click START SHIFT     P pause", 240.0, 250.0, 5.5, DIM);
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_week_shows_its_kicker_budget_and_quota() {
        let s = ShopState::new(3, 0, false);
        assert_eq!(s.kicker(), "DAY \u{b7} WEEK 3");
        assert_eq!(s.budget(), 110);
        assert_eq!(s.quota(), 280);
        assert_eq!(s.budget_line(), "Budget $110 of $110");
        assert_eq!(s.quota_line(), "Tonight's quota $280");
        let c = ShopState::new(2, 25, false);
        assert_eq!(c.budget_line(), "Budget $125 of $125 (incl. $25 carried over)");
    }

    #[test]
    fn a_custom_night_uses_week_four_tables() {
        let s = ShopState::new(1, 0, true);
        assert_eq!(s.kicker(), "DAY \u{b7} CUSTOM");
        assert_eq!(s.table_week(), 4);
        assert_eq!(s.budget(), 120);
        assert_eq!(s.quota(), 340);
    }

    #[test]
    fn toggling_spends_the_budget_and_refuses_past_it() {
        let mut s = ShopState::new(1, 0, false); // $90
        assert!(s.toggle("gloves")); // 45
        assert!(s.toggle("soles")); // 35 -> 80
        assert_eq!(s.left(), 10);
        assert!(!s.toggle("bench"), "40 does not fit");
        assert!(!s.has("bench"));
        assert!(s.toggle("gloves"), "taking one back always works");
        assert_eq!(s.left(), 55);
        assert!(s.toggle("bench"));
        assert_eq!(s.owned, vec!["soles".to_string(), "bench".to_string()]);
    }

    #[test]
    fn rows_grey_out_what_cannot_be_afforded_and_tick_what_is_owned() {
        let mut s = ShopState::new(1, 0, false);
        s.toggle("gloves");
        s.toggle("soles");
        let rows = s.rows();
        assert_eq!(rows.len(), 5);
        let by = |id: &str| rows.iter().find(|r| r.id == id).cloned().expect("row");
        assert!(by("gloves").has && !by("gloves").disabled);
        assert!(by("bench").disabled && !by("bench").has);
        assert!(by("cell").disabled);
        assert!(!by("hinges").disabled || by("hinges").text.contains("$30"));
        assert!(by("gloves").text.starts_with('\u{2714}'));
        assert!(by("soles").text.contains("SOFT SOLES") && by("soles").text.contains("$35"));
        assert!(by("bench").text.starts_with(' '));
    }

    #[test]
    fn the_carry_buys_one_more_upgrade() {
        let mut s = ShopState::new(2, 40, false); // 100 + 40
        assert!(s.toggle("gloves") && s.toggle("soles") && s.toggle("bench") && s.toggle("hinges") == false);
        assert_eq!(s.left(), 140 - 120);
    }

    #[test]
    fn the_button_rectangles_do_not_overlap() {
        for i in 0..5 {
            let r = row_rect(i);
            assert!(r.x + r.w <= PANEL.x + PANEL.w && r.y + r.h <= start_rect().y);
            if i > 0 {
                assert!(r.y >= row_rect(i - 1).y + row_rect(i - 1).h);
            }
        }
        let s = start_rect();
        assert!(s.y + s.h <= PANEL.y + PANEL.h);
    }
}
