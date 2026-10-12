//! Ktore zmiany stanu VPN zasluguja na powiadomienie. Czysta logika, niezalezna od UI i D-Bus.

use crate::systemd::VpnState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Connected,
    /// Polaczenie zerwane, systemd laczy ponownie.
    Reconnecting,
    /// Polaczenie zamkniete poza apletem.
    Disconnected,
    /// Nieudana proba (ponawiana) albo stan `failed`; tresc (powod) dopiero z logu.
    Failure,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Tracker {
    /// Uzytkownik rozlacza z apletu - zejscie z Connected nie jest wtedy "utrata polaczenia".
    user_stop: bool,
    /// Polaczenie zerwane bez udzialu uzytkownika; powiadomienie czeka na stan docelowy.
    lost: bool,
    /// Powiadomienie o nieudanej probie juz poszlo w tej serii ponowien.
    retry_notified: bool,
}

impl Tracker {
    pub fn stop_requested(&mut self) {
        self.user_stop = true;
    }

    /// Zatrzymanie sie nie udalo (np. anulowany polkit) - pozniejsze zerwanie to znow "utrata".
    pub fn stop_failed(&mut self) {
        self.user_stop = false;
    }

    /// Zdarzenie dla zmiany `prev` -> `state`. `retry_started`: unit wlasnie wszedl w auto-restart
    /// (VpnState moze sie przy tym nie zmienic).
    pub fn transition(
        &mut self,
        prev: VpnState,
        state: VpnState,
        retry_started: bool,
    ) -> Option<Event> {
        if prev == VpnState::Connected && state != VpnState::Connected && !self.user_stop {
            self.lost = true;
        }
        let event = match state {
            VpnState::Connected => {
                self.lost = false;
                self.retry_notified = false;
                Some(Event::Connected)
            }
            VpnState::Connecting if std::mem::take(&mut self.lost) => Some(Event::Reconnecting),
            VpnState::Inactive => {
                self.user_stop = false;
                self.retry_notified = false;
                std::mem::take(&mut self.lost).then_some(Event::Disconnected)
            }
            VpnState::Failed => {
                self.lost = false;
                self.user_stop = false;
                self.retry_notified = false;
                Some(Event::Failure)
            }
            VpnState::Connecting | VpnState::Disconnecting => None,
        };
        // pierwsza nieudana proba w serii, chyba ze wlasnie poszlo inne powiadomienie
        if event.is_none() && retry_started && !self.retry_notified {
            self.retry_notified = true;
            return Some(Event::Failure);
        }
        event
    }
}

#[cfg(test)]
mod tests {
    use super::Event::*;
    use super::*;
    use VpnState::{Connected as Up, Connecting, Disconnecting, Failed, Inactive};

    /// Przechodzi przez kolejne (stan, retry_started) i zbiera zdarzenia.
    fn run(t: &mut Tracker, start: VpnState, steps: &[(VpnState, bool)]) -> Vec<Option<Event>> {
        let mut prev = start;
        steps
            .iter()
            .map(|&(state, retry)| {
                let e = t.transition(prev, state, retry);
                prev = state;
                e
            })
            .collect()
    }

    #[test]
    fn connect() {
        let mut t = Tracker::default();
        let ev = run(&mut t, Inactive, &[(Connecting, false), (Up, false)]);
        assert_eq!(ev, [None, Some(Connected)]);
    }

    #[test]
    fn user_disconnect_is_silent() {
        let mut t = Tracker::default();
        t.stop_requested();
        let ev = run(&mut t, Up, &[(Disconnecting, false), (Inactive, false)]);
        assert_eq!(ev, [None, None]);
        // flaga wyczyszczona: kolejne zerwanie znow powiadamia
        let ev = run(&mut t, Up, &[(Disconnecting, false), (Inactive, false)]);
        assert_eq!(ev, [None, Some(Disconnected)]);
    }

    #[test]
    fn cancelled_stop_still_reports_drop() {
        let mut t = Tracker::default();
        t.stop_requested();
        t.stop_failed();
        let ev = run(&mut t, Up, &[(Disconnecting, false), (Connecting, true)]);
        assert_eq!(ev, [None, Some(Reconnecting)]);
    }

    #[test]
    fn drop_and_reconnect() {
        let mut t = Tracker::default();
        let ev = run(
            &mut t,
            Up,
            &[(Disconnecting, false), (Connecting, true), (Up, false)],
        );
        assert_eq!(ev, [None, Some(Reconnecting), Some(Connected)]);
    }

    #[test]
    fn failed_attempts_notify_once_then_failed() {
        let mut t = Tracker::default();
        let ev = run(
            &mut t,
            Inactive,
            &[
                (Connecting, false),
                (Connecting, true),
                (Connecting, false),
                (Connecting, true),
                (Failed, false),
            ],
        );
        assert_eq!(ev, [None, Some(Failure), None, None, Some(Failure)]);
        // nowa seria po bledzie znow powiadamia o pierwszej porazce
        let ev = run(&mut t, Failed, &[(Connecting, false), (Connecting, true)]);
        assert_eq!(ev, [None, Some(Failure)]);
    }
}
