use crate::settings::{Cleanup, EnterAction, OnPick, Settings, Theme};

pub const QUESTIONS: usize = 9;
pub const DONE: usize = QUESTIONS + 1;
const THEME: usize = 0;
const WEEK_DAYS: u32 = 7;
const LIMIT_MB: u32 = 1024;

/// What moving forward did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Moved,
    Concluded,
    OpenSearch,
}

/// Step 0 is the welcome, 1 to 9 the questions (in the spec's order) and 10 the last screen.
/// An answer is the index of the chosen option, where 0 is always the factory default;
/// `None` keeps the current value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Onboarding {
    pub step: usize,
    pub answers: [Option<usize>; QUESTIONS],
    pub upgrade: bool,
}

impl Onboarding {
    pub fn new(settings: &Settings, upgrade: bool) -> Self {
        Self {
            step: 0,
            answers: current_answers(settings),
            upgrade,
        }
    }

    fn question(&self) -> Option<usize> {
        (1..=QUESTIONS).contains(&self.step).then(|| self.step - 1)
    }

    /// The selected option of the current question: -1 outside the questions or when none
    /// matches the current value.
    pub fn selected(&self) -> i32 {
        self.question()
            .and_then(|question| self.answers[question])
            .map_or(-1, |option| option as i32)
    }

    /// Answers the current question; returns whether it was the theme question.
    pub fn select(&mut self, option: usize) -> bool {
        match self.question() {
            Some(question) if option < 2 => {
                self.answers[question] = Some(option);
                question == THEME
            }
            _ => false,
        }
    }

    pub fn next(&mut self) -> Outcome {
        match self.step {
            DONE => Outcome::OpenSearch,
            QUESTIONS => {
                self.step = DONE;
                Outcome::Concluded
            }
            _ => {
                self.step += 1;
                Outcome::Moved
            }
        }
    }

    pub fn back(&mut self) {
        if self.question().is_some() {
            self.step -= 1;
        }
    }

    /// Jumps to the last screen; returns whether this concluded the onboarding.
    pub fn skip(&mut self) -> bool {
        let concluded = self.step != DONE;
        self.step = DONE;
        concluded
    }

    pub fn progress(&self) -> f32 {
        self.step.min(QUESTIONS) as f32 / QUESTIONS as f32
    }

    /// The theme to preview: the answer, or `saved` while the answer keeps the current value.
    pub fn theme(&self, saved: Theme) -> Theme {
        match self.answers[THEME] {
            Some(0) => Theme::Light,
            Some(_) => Theme::Dark,
            None => saved,
        }
    }

    /// Takes the settings changed elsewhere from `old` to `new` as the answers to their
    /// questions, so concluding does not overwrite them.
    pub fn sync(&mut self, old: &Settings, new: &Settings) {
        let answers = current_answers(new);
        for (question, answer) in self.answers.iter_mut().enumerate() {
            if fields(old, question) != fields(new, question) {
                *answer = answers[question];
            }
        }
    }

    /// Writes the answered questions over `settings` and marks the onboarding done.
    pub fn apply(&self, settings: &mut Settings) {
        for (question, answer) in self.answers.iter().enumerate() {
            if let Some(option) = *answer {
                write(settings, question, option == 0);
            }
        }
        settings.general.onboarding_done = true;
        settings.normalize();
    }
}

fn current_answers(settings: &Settings) -> [Option<usize>; QUESTIONS] {
    let general = &settings.general;
    let search = &settings.search;
    let clipboard = &settings.clipboard;
    let first = |yes: bool| Some(if yes { 0 } else { 1 });
    [
        match general.theme {
            Theme::Light => Some(0),
            Theme::Dark => Some(1),
            Theme::System => None,
        },
        first(general.windows_key),
        first(general.start_with_windows),
        first(general.auto_update),
        first(search.enter_action == EnterAction::Open),
        first(search.include_hidden),
        first(!clipboard.paused),
        first(clipboard.on_pick == OnPick::Copy),
        match clipboard.cleanup {
            Cleanup::Time if clipboard.cleanup_days == WEEK_DAYS => Some(0),
            Cleanup::Storage if clipboard.cleanup_limit_mb == LIMIT_MB => Some(1),
            _ => None,
        },
    ]
}

fn fields(settings: &Settings, question: usize) -> (u32, u32, u32) {
    let general = &settings.general;
    let search = &settings.search;
    let clipboard = &settings.clipboard;
    match question {
        0 => (general.theme as u32, 0, 0),
        1 => (general.windows_key.into(), 0, 0),
        2 => (general.start_with_windows.into(), 0, 0),
        3 => (general.auto_update.into(), 0, 0),
        4 => (search.enter_action as u32, 0, 0),
        5 => (search.include_hidden.into(), 0, 0),
        6 => (clipboard.paused.into(), 0, 0),
        7 => (clipboard.on_pick as u32, 0, 0),
        _ => (
            clipboard.cleanup as u32,
            clipboard.cleanup_days,
            clipboard.cleanup_limit_mb,
        ),
    }
}

fn write(settings: &mut Settings, question: usize, first: bool) {
    let general = &mut settings.general;
    let search = &mut settings.search;
    let clipboard = &mut settings.clipboard;
    match question {
        0 => general.theme = if first { Theme::Light } else { Theme::Dark },
        1 => general.windows_key = first,
        2 => general.start_with_windows = first,
        3 => general.auto_update = first,
        4 => {
            search.enter_action = if first {
                EnterAction::Open
            } else {
                EnterAction::Reveal
            }
        }
        5 => search.include_hidden = first,
        6 => clipboard.paused = !first,
        7 => {
            clipboard.on_pick = if first {
                OnPick::Copy
            } else {
                OnPick::CopyAndPaste
            }
        }
        _ if first => {
            clipboard.cleanup = Cleanup::Time;
            clipboard.cleanup_days = WEEK_DAYS;
        }
        _ => {
            clipboard.cleanup = Cleanup::Storage;
            clipboard.cleanup_limit_mb = LIMIT_MB;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_step(step: usize) -> Onboarding {
        let mut onboarding = Onboarding::new(&Settings::default(), false);
        while onboarding.step < step {
            onboarding.next();
        }
        onboarding
    }

    #[test]
    fn a_new_install_starts_with_every_default_selected() {
        let onboarding = Onboarding::new(&Settings::default(), false);
        assert_eq!(onboarding.step, 0);
        assert_eq!(onboarding.answers, [Some(0); QUESTIONS]);
        assert_eq!(onboarding.selected(), -1);
        assert_eq!(at_step(1).selected(), 0);
    }

    #[test]
    fn moves_through_the_steps() {
        let mut onboarding = Onboarding::new(&Settings::default(), false);
        onboarding.back();
        assert_eq!(onboarding.step, 0);
        for step in 1..=QUESTIONS {
            assert_eq!(onboarding.next(), Outcome::Moved);
            assert_eq!(onboarding.step, step);
        }
        assert_eq!(onboarding.next(), Outcome::Concluded);
        assert_eq!(onboarding.step, DONE);
        onboarding.back();
        assert_eq!(onboarding.step, DONE);
        assert_eq!(onboarding.next(), Outcome::OpenSearch);
        assert_eq!(onboarding.step, DONE);
    }

    #[test]
    fn going_back_keeps_the_answer() {
        let mut onboarding = at_step(2);
        onboarding.select(1);
        onboarding.next();
        onboarding.back();
        assert_eq!(onboarding.step, 2);
        assert_eq!(onboarding.selected(), 1);
    }

    #[test]
    fn select_only_answers_the_current_question() {
        let mut onboarding = Onboarding::new(&Settings::default(), false);
        assert!(!onboarding.select(1));
        assert_eq!(onboarding.answers, [Some(0); QUESTIONS]);
        onboarding.next();
        assert!(onboarding.select(1));
        assert_eq!(onboarding.theme(Theme::System), Theme::Dark);
        onboarding.next();
        assert!(!onboarding.select(1));
        assert!(!onboarding.select(5));
        assert_eq!(onboarding.answers[1], Some(1));
    }

    #[test]
    fn skip_keeps_the_answers_given() {
        let mut onboarding = at_step(3);
        onboarding.select(1);
        assert!(onboarding.skip());
        assert_eq!(onboarding.step, DONE);
        assert_eq!(onboarding.answers[2], Some(1));
        assert!(!onboarding.skip());
    }

    #[test]
    fn progress_fills_up_to_the_last_question() {
        assert_eq!(at_step(0).progress(), 0.0);
        assert!((at_step(3).progress() - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(at_step(QUESTIONS).progress(), 1.0);
        assert_eq!(at_step(DONE).progress(), 1.0);
    }

    #[test]
    fn pressing_enter_everywhere_keeps_the_factory_settings() {
        let mut settings = Settings::default();
        let mut onboarding = Onboarding::new(&settings, false);
        while onboarding.next() != Outcome::OpenSearch {}
        onboarding.apply(&mut settings);
        let mut expected = Settings::default();
        expected.general.onboarding_done = true;
        assert_eq!(settings, expected);
    }

    #[test]
    fn second_options_write_every_setting() {
        let mut settings = Settings::default();
        let mut onboarding = Onboarding::new(&settings, false);
        for _ in 0..QUESTIONS {
            onboarding.next();
            onboarding.select(1);
        }
        onboarding.apply(&mut settings);
        let general = &settings.general;
        assert_eq!(general.theme, Theme::Dark);
        assert!(!general.windows_key && !general.start_with_windows && !general.auto_update);
        assert!(general.onboarding_done);
        assert_eq!(settings.search.enter_action, EnterAction::Reveal);
        assert!(!settings.search.include_hidden);
        let clipboard = &settings.clipboard;
        assert!(clipboard.paused);
        assert_eq!(clipboard.on_pick, OnPick::CopyAndPaste);
        assert_eq!(clipboard.cleanup, Cleanup::Storage);
        assert_eq!(clipboard.cleanup_limit_mb, 1024);
    }

    #[test]
    fn values_without_a_matching_option_are_kept() {
        let mut settings = Settings::default();
        settings.general.theme = Theme::System;
        settings.general.windows_key = false;
        settings.clipboard.cleanup_days = 3;
        let onboarding = Onboarding::new(&settings, true);
        assert!(onboarding.upgrade);
        assert_eq!(onboarding.answers[0], None);
        assert_eq!(onboarding.answers[1], Some(1));
        assert_eq!(onboarding.answers[8], None);
        let mut applied = settings.clone();
        onboarding.apply(&mut applied);
        assert_eq!(applied.general.theme, Theme::System);
        assert!(!applied.general.windows_key);
        assert_eq!(applied.clipboard.cleanup_days, 3);
        assert!(applied.general.onboarding_done);
        assert_eq!(onboarding.theme(Theme::System), Theme::System);

        settings.clipboard.cleanup = Cleanup::Storage;
        settings.clipboard.cleanup_limit_mb = 2048;
        assert_eq!(Onboarding::new(&settings, true).answers[8], None);
        settings.clipboard.cleanup_limit_mb = 1024;
        assert_eq!(Onboarding::new(&settings, true).answers[8], Some(1));
    }

    #[test]
    fn sync_takes_only_the_settings_changed_elsewhere() {
        let old = Settings::default();
        let mut onboarding = at_step(3);
        onboarding.select(1);
        onboarding.back();
        onboarding.select(1);
        let mut new = old.clone();
        new.general.theme = Theme::System;
        new.clipboard.paused = true;
        new.clipboard.cleanup_days = 3;
        new.general.language = crate::settings::Language::En;
        onboarding.sync(&old, &new);
        let mut expected = [Some(0); QUESTIONS];
        expected[0] = None;
        expected[1] = Some(1);
        expected[2] = Some(1);
        expected[6] = Some(1);
        expected[8] = None;
        assert_eq!(onboarding.answers, expected);
        assert_eq!(onboarding.step, 2);
        onboarding.sync(&new, &new);
        assert_eq!(onboarding.answers, expected);
    }

    #[test]
    fn sync_overrides_an_answer_when_the_setting_changes() {
        let mut old = Settings::default();
        old.clipboard.cleanup_days = 3;
        let mut onboarding = Onboarding::new(&old, false);
        onboarding.answers[8] = Some(1);
        let mut new = old.clone();
        new.clipboard.cleanup_days = 30;
        onboarding.sync(&old, &new);
        assert_eq!(onboarding.answers[8], None);
        onboarding.answers[8] = Some(1);
        let mut later = new.clone();
        later.clipboard.cleanup_days = 7;
        onboarding.sync(&new, &later);
        assert_eq!(onboarding.answers[8], Some(0));
        let mut applied = later.clone();
        onboarding.apply(&mut applied);
        assert_eq!(applied.clipboard.cleanup, Cleanup::Time);
        assert_eq!(applied.clipboard.cleanup_days, 7);
    }
}
