//! Our own dialogs: Lucker Party Options (the settings CS:S's options
//! don't have) and, in a game, Bots and Team.

use super::*;

impl GameMenu {
    /// Lucker Party Options' rows: ours, OK, Cancel.
    pub(super) fn extras_rows(&self) -> Vec<Row> {
        let mut rows = self.place_rows(Place::Extras);
        for (i, s) in SETTINGS.iter().enumerate() {
            if s.place == Place::Extras && self.values.get(i).cloned().flatten().is_none() {
                rows.push(Row::Info(format!("{}: not available", s.label)));
            }
        }
        rows.push(self.ok_row());
        rows.push(self.cancel_row());
        rows
    }

    /// The Bots dialog's rows (ours).
    pub(super) fn bots_rows(&self) -> Vec<Row> {
        vec![
            Row::Info(format!(
                "In the game: {} terrorist and {} counter-terrorist bots",
                self.bots[0], self.bots[1]
            )),
            Self::button_row("Add a terrorist bot", Action::Run("bot_add 1".into(), false)),
            Self::button_row("Add a counter-terrorist bot", Action::Run("bot_add 2".into(), false)),
            Self::button_row("Kick all bots", Action::Run("bot_kick".into(), false)),
            Self::button_row(&self.text("#GameUI_Close", "Close"), Action::Back),
        ]
    }

    /// The Team dialog's rows (ours).
    pub(super) fn team_rows(&self) -> Vec<Row> {
        vec![
            Self::button_row("Terrorists", Action::Run("jointeam 2".into(), true)),
            Self::button_row("Counter-Terrorists", Action::Run("jointeam 3".into(), true)),
            Self::button_row(
                "Auto-assign",
                Action::Run(
                    format!(
                        "jointeam {}",
                        crate::client::team_menu::auto_team(self.players[0], self.players[1])
                    ),
                    true,
                ),
            ),
            // CS:S's team menu offers it too (`jointeam 1`); greyed when
            // the server's `mp_allowspectators` is 0.
            Row::Button {
                label: "Spectate".into(),
                action: Action::Run("jointeam 1".into(), true),
                enabled: !self.no_spectators,
            },
            Self::button_row(&self.text("#GameUI_Cancel", "Cancel"), Action::Back),
        ]
    }
}
