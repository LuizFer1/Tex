use crate::settings::{Cleanup, EnterAction, OnPick, Settings, Theme};

pub const QUESTIONS: usize = 9;
const WEEK_DAYS: u32 = 7;
const LIMIT_MB: u32 = 1024;

/// One onboarding question. Its position in [`ALL`] is its index in the answers and in the
/// question list of `ui/onboarding.slint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Question {
    Theme,
    WindowsKey,
    StartWithSystem,
    Updates,
    EnterAction,
    Hidden,
    Capture,
    OnPick,
    Cleanup,
}

/// Every question, in the spec's order: what Windows asks.
pub const ALL: [Question; QUESTIONS] = [
    Question::Theme,
    Question::WindowsKey,
    Question::StartWithSystem,
    Question::Updates,
    Question::EnterAction,
    Question::Hidden,
    Question::Capture,
    Question::OnPick,
    Question::Cleanup,
];

/// Linux has no Windows key question: the Super key belongs to the desktop.
pub const LINUX: [Question; 8] = [
    Question::Theme,
    Question::StartWithSystem,
    Question::Updates,
    Question::EnterAction,
    Question::Hidden,
    Question::Capture,
    Question::OnPick,
    Question::Cleanup,
];

/// The questions this platform asks.
pub fn platform_questions() -> &'static [Question] {
    if cfg!(target_os = "linux") {
        &LINUX
    } else {
        &ALL
    }
}

/// What moving forward did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Moved,
    Concluded,
    OpenSearch,
}

/// Step 0 is the welcome, 1 to `count()` the platform's questions and `done()` the last
/// screen. An answer is the index of the chosen option, where 0 is always the factory
/// default; `None` keeps the current value until the question is left with `next`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Onboarding {
    pub step: usize,
    pub answers: [Option<usize>; QUESTIONS],
    pub upgrade: bool,
    questions: &'static [Question],
}

impl Onboarding {
    pub fn new(settings: &Settings, upgrade: bool) -> Self {
        Self::with_questions(settings, upgrade, platform_questions())
    }

    pub fn with_questions(
        settings: &Settings,
        upgrade: bool,
        questions: &'static [Question],
    ) -> Self {
        Self {
            step: 0,
            answers: current_answers(settings),
            upgrade,
            questions,
        }
    }

    pub fn count(&self) -> usize {
        self.questions.len()
    }

    pub fn done(&self) -> usize {
        self.count() + 1
    }

    fn question(&self) -> Option<Question> {
        (1..=self.count())
            .contains(&self.step)
            .then(|| self.questions[self.step - 1])
    }

    /// Index of the current question in [`ALL`] (0 outside the questions), which is where
    /// `ui/onboarding.slint` keeps its texts.
    pub fn question_id(&self) -> i32 {
        self.question().map_or(0, |question| question as i32)
    }

    /// The selected option of the current question: -1 outside the questions, and the default
    /// while the answer keeps the current value.
    pub fn selected(&self) -> i32 {
        self.question().map_or(-1, |question| {
            self.answers[question as usize].unwrap_or(0) as i32
        })
    }

    /// Answers the current question; returns whether it was the theme question.
    pub fn select(&mut self, option: usize) -> bool {
        match self.question() {
            Some(question) if option < 2 => {
                self.answers[question as usize] = Some(option);
                question == Question::Theme
            }
            _ => false,
        }
    }

    /// Moves forward, committing the default shown on a question that keeps the current value.
    pub fn next(&mut self) -> Outcome {
        if let Some(question) = self.question() {
            self.answers[question as usize].get_or_insert(0);
        }
        if self.step == self.done() {
            Outcome::OpenSearch
        } else if self.step == self.count() {
            self.step = self.done();
            Outcome::Concluded
        } else {
            self.step += 1;
            Outcome::Moved
        }
    }

    pub fn back(&mut self) {
        if self.question().is_some() {
            self.step -= 1;
        }
    }

    /// Jumps to the last screen; returns whether this concluded the onboarding.
    pub fn skip(&mut self) -> bool {
        let concluded = self.step != self.done();
        self.step = self.done();
        concluded
    }

    pub fn progress(&self) -> f32 {
        self.step.min(self.count()) as f32 / self.count() as f32
    }

    /// The theme to preview: the answer, or `saved` while the answer keeps the current value.
    pub fn theme(&self, saved: Theme) -> Theme {
        match self.answers[Question::Theme as usize] {
            Some(0) => Theme::Light,
            Some(_) => Theme::Dark,
            None => saved,
        }
    }

    /// Takes the settings changed elsewhere from `old` to `new` as the answers to their
    /// questions, so concluding does not overwrite them.
    pub fn sync(&mut self, old: &Settings, new: &Settings) {
        let answers = current_answers(new);
        for question in self.questions {
            if fields(old, *question) != fields(new, *question) {
                self.answers[*question as usize] = answers[*question as usize];
            }
        }
    }

    /// Writes the answered questions over `settings` and marks the onboarding done. Questions
    /// this platform does not ask are left alone.
    pub fn apply(&self, settings: &mut Settings) {
        for question in self.questions {
            if let Some(option) = self.answers[*question as usize] {
                write(settings, *question, option == 0);
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

fn fields(settings: &Settings, question: Question) -> (u32, u32, u32) {
    let general = &settings.general;
    let search = &settings.search;
    let clipboard = &settings.clipboard;
    match question {
        Question::Theme => (general.theme as u32, 0, 0),
        Question::WindowsKey => (general.windows_key.into(), 0, 0),
        Question::StartWithSystem => (general.start_with_windows.into(), 0, 0),
        Question::Updates => (general.auto_update.into(), 0, 0),
        Question::EnterAction => (search.enter_action as u32, 0, 0),
        Question::Hidden => (search.include_hidden.into(), 0, 0),
        Question::Capture => (clipboard.paused.into(), 0, 0),
        Question::OnPick => (clipboard.on_pick as u32, 0, 0),
        Question::Cleanup => (
            clipboard.cleanup as u32,
            clipboard.cleanup_days,
            clipboard.cleanup_limit_mb,
        ),
    }
}

fn write(settings: &mut Settings, question: Question, first: bool) {
    let general = &mut settings.general;
    let search = &mut settings.search;
    let clipboard = &mut settings.clipboard;
    match question {
        Question::Theme => general.theme = if first { Theme::Light } else { Theme::Dark },
        Question::WindowsKey => general.windows_key = first,
        Question::StartWithSystem => general.start_with_windows = first,
        Question::Updates => general.auto_update = first,
        Question::EnterAction => {
            search.enter_action = if first {
                EnterAction::Open
            } else {
                EnterAction::Reveal
            }
        }
        Question::Hidden => search.include_hidden = first,
        Question::Capture => clipboard.paused = !first,
        Question::OnPick => {
            clipboard.on_pick = if first {
                OnPick::Copy
            } else {
                OnPick::CopyAndPaste
            }
        }
        Question::Cleanup if first => {
            clipboard.cleanup = Cleanup::Time;
            clipboard.cleanup_days = WEEK_DAYS;
        }
        Question::Cleanup => {
            clipboard.cleanup = Cleanup::Storage;
            clipboard.cleanup_limit_mb = LIMIT_MB;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DONE: usize = QUESTIONS + 1;

    fn at_step(step: usize) -> Onboarding {
        let mut onboarding = Onboarding::with_questions(&Settings::default(), false, &ALL);
        while onboarding.step < step {
            onboarding.next();
        }
        onboarding
    }

    #[test]
    fn a_new_install_starts_with_every_default_selected() {
        let onboarding = Onboarding::with_questions(&Settings::default(), false, &ALL);
        assert_eq!(onboarding.step, 0);
        assert_eq!(onboarding.answers, [Some(0); QUESTIONS]);
        assert_eq!(onboarding.selected(), -1);
        assert_eq!(at_step(1).selected(), 0);
    }

    #[test]
    fn moves_through_the_steps() {
        let mut onboarding = Onboarding::with_questions(&Settings::default(), false, &ALL);
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
        let mut onboarding = Onboarding::with_questions(&Settings::default(), false, &ALL);
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
        let mut onboarding = Onboarding::with_questions(&settings, false, &ALL);
        while onboarding.next() != Outcome::OpenSearch {}
        onboarding.apply(&mut settings);
        let mut expected = Settings::default();
        expected.general.onboarding_done = true;
        assert_eq!(settings, expected);
    }

    #[test]
    fn second_options_write_every_setting() {
        let mut settings = Settings::default();
        let mut onboarding = Onboarding::with_questions(&settings, false, &ALL);
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
        let onboarding = Onboarding::with_questions(&settings, true, &ALL);
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
        assert_eq!(
            Onboarding::with_questions(&settings, true, &ALL).answers[8],
            None
        );
        settings.clipboard.cleanup_limit_mb = 1024;
        assert_eq!(
            Onboarding::with_questions(&settings, true, &ALL).answers[8],
            Some(1)
        );
    }

    fn without_matching_options() -> Settings {
        let mut settings = Settings::default();
        settings.general.theme = Theme::System;
        settings.clipboard.cleanup_days = 30;
        settings
    }

    #[test]
    fn a_value_without_a_matching_option_shows_the_default_selected() {
        let mut onboarding = Onboarding::with_questions(&without_matching_options(), true, &ALL);
        onboarding.next();
        assert_eq!(onboarding.selected(), 0);
        assert_eq!(onboarding.answers[0], None);
        while onboarding.step < QUESTIONS {
            onboarding.next();
        }
        assert_eq!(onboarding.selected(), 0);
        assert_eq!(onboarding.answers[8], None);
    }

    #[test]
    fn leaving_a_question_with_next_commits_the_default_shown() {
        let settings = without_matching_options();
        let mut onboarding = Onboarding::with_questions(&settings, true, &ALL);
        onboarding.next();
        assert_eq!(onboarding.theme(Theme::System), Theme::System);
        assert_eq!(onboarding.next(), Outcome::Moved);
        assert_eq!(onboarding.answers[0], Some(0));
        assert_eq!(onboarding.theme(Theme::System), Theme::Light);
        while onboarding.step < QUESTIONS {
            onboarding.next();
        }
        assert_eq!(onboarding.next(), Outcome::Concluded);
        assert_eq!(onboarding.answers[8], Some(0));
        let mut applied = settings.clone();
        onboarding.apply(&mut applied);
        assert_eq!(applied.general.theme, Theme::Light);
        assert_eq!(applied.clipboard.cleanup, Cleanup::Time);
        assert_eq!(applied.clipboard.cleanup_days, WEEK_DAYS);
    }

    #[test]
    fn skip_leaves_unvisited_questions_with_the_current_value() {
        let settings = without_matching_options();
        let mut onboarding = Onboarding::with_questions(&settings, true, &ALL);
        onboarding.next();
        onboarding.next();
        assert!(onboarding.skip());
        assert_eq!(onboarding.answers[0], Some(0));
        assert_eq!(onboarding.answers[8], None);
        let mut applied = settings.clone();
        onboarding.apply(&mut applied);
        assert_eq!(applied.general.theme, Theme::Light);
        assert_eq!(applied.clipboard.cleanup_days, 30);
    }

    #[test]
    fn skip_on_a_question_shown_with_the_default_keeps_the_current_value() {
        let settings = without_matching_options();
        let mut onboarding = Onboarding::with_questions(&settings, true, &ALL);
        onboarding.next();
        assert_eq!(onboarding.selected(), 0);
        assert!(onboarding.skip());
        assert_eq!(onboarding.answers[0], None);
        let mut applied = settings.clone();
        onboarding.apply(&mut applied);
        assert_eq!(applied.general.theme, Theme::System);
    }

    #[test]
    fn welcome_and_last_screen_show_nothing_selected() {
        let onboarding = Onboarding::with_questions(&without_matching_options(), true, &ALL);
        assert_eq!(onboarding.selected(), -1);
        assert_eq!(at_step(DONE).selected(), -1);
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
        let mut onboarding = Onboarding::with_questions(&old, false, &ALL);
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

    #[test]
    fn linux_asks_eight_questions_without_the_windows_key() {
        assert!(!LINUX.contains(&Question::WindowsKey));
        let mut onboarding = Onboarding::with_questions(&Settings::default(), false, &LINUX);
        assert_eq!(onboarding.count(), 8);
        assert_eq!(onboarding.next(), Outcome::Moved);
        assert_eq!(onboarding.question_id(), Question::Theme as i32);
        assert_eq!(onboarding.next(), Outcome::Moved);
        assert_eq!(onboarding.question_id(), Question::StartWithSystem as i32);
        while onboarding.step < 8 {
            assert_eq!(onboarding.next(), Outcome::Moved);
        }
        assert_eq!(onboarding.question_id(), Question::Cleanup as i32);
        assert_eq!(onboarding.next(), Outcome::Concluded);
        assert_eq!(onboarding.step, 9);
        assert_eq!(onboarding.done(), 9);
        assert_eq!(onboarding.next(), Outcome::OpenSearch);
    }

    #[test]
    fn linux_never_writes_the_windows_key() {
        let mut settings = Settings::default();
        let mut onboarding = Onboarding::with_questions(&settings, false, &LINUX);
        onboarding.answers[Question::WindowsKey as usize] = Some(1);
        for _ in 0..LINUX.len() {
            onboarding.next();
            onboarding.select(1);
        }
        onboarding.apply(&mut settings);
        assert!(settings.general.windows_key);
        assert!(!settings.general.start_with_windows);
        assert_eq!(settings.clipboard.cleanup, Cleanup::Storage);
    }

    #[test]
    fn progress_counts_the_questions_of_the_list() {
        let mut onboarding = Onboarding::with_questions(&Settings::default(), false, &LINUX);
        for _ in 0..4 {
            onboarding.next();
        }
        assert_eq!(onboarding.progress(), 0.5);
    }

    #[test]
    fn this_platform_asks_its_own_list() {
        let expected: &[Question] = if cfg!(target_os = "linux") {
            &LINUX
        } else {
            &ALL
        };
        assert_eq!(platform_questions(), expected);
        assert_eq!(
            Onboarding::new(&Settings::default(), false).count(),
            expected.len()
        );
    }

    #[test]
    fn question_ids_follow_the_full_list() {
        for (index, question) in ALL.iter().enumerate() {
            assert_eq!(*question as usize, index);
        }
        let onboarding = Onboarding::with_questions(&Settings::default(), false, &ALL);
        assert_eq!(onboarding.question_id(), 0);
    }
}
