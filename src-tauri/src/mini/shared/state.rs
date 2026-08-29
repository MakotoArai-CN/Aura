//! 轻量模式的播放状态机。不含任何平台类型，也不直接碰音频实现。
//!
//! 状态机只管"现在该做什么"，具体怎么做由调用方执行：`begin_current` 之类的方法返回
//! 一串 [`Action`]，调用方按顺序执行，再把装载结果通过 [`Playback::on_load_ok`] /
//! [`Playback::on_load_failed`] 报回来。这样同一份换曲、循环、洗牌、失败退避的逻辑
//! 能被 Direct2D / Cairo / CoreGraphics 三个后端共用，各自只需要接上自己的音频输出。
//!
//! 地址解析（`playable_uri`）刻意留在调用方：它要查磁盘缓存，放进来这个模块就没法
//! 在单元测试里脱离文件系统跑了。

use super::super::MiniSnapshot;

/// 连挂几首就停手。这种情况基本是签名直链集体过期，再往后跳也是同样的结果。
const MAX_FAILURES: u32 = 3;

/// 恢复进度的门槛。半秒以内当作从头开始，没必要为了这点差别多一次 seek。
const RESUME_THRESHOLD: f64 = 0.5;

/// 等源打开的最多帧数。100ms 一帧，也就是一秒；再等不到就从头播，总比不播好。
const PENDING_SEEK_TRIES: u32 = 10;

/// 顺序播放。
pub const LOOP_SEQUENCE: u8 = 0;
/// 单曲循环。
pub const LOOP_ONE: u8 = 1;
/// 随机播放。
pub const LOOP_SHUFFLE: u8 = 2;

/// 状态机要求调用方执行的动作。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    SetVolume(f64),
    SetMuted(bool),
    /// 装载第 `index` 首。调用方负责解析出可播地址，解析不出来就当作装载失败。
    Load { index: i64 },
    Play,
    Pause,
    /// 停下并显示提示，不再继续往后跳。
    Halt(&'static str),
}

/// 播放状态机：队列、下标、循环模式、音量、失败退避、洗牌、待补的 seek。
#[derive(Debug)]
pub struct Playback {
    snapshot: MiniSnapshot,
    /// 连续装载失败次数。成功一次就归零。
    failures: u32,
    /// 停下来的原因。非 None 时界面显示提示，再点播放当作重试。
    notice: Option<&'static str>,
    /// 拖动进度条时的本地预览位置。松手才真的 seek，免得一路拖一路重新缓冲。
    scrub: Option<f64>,
    /// 恢复进度：刚 load 完源还在打开中，这时 seek 会被丢掉，得等它开好。
    pending_seek: Option<(f64, u32)>,
    /// 随机播放用的 xorshift 种子。为一个洗牌引一个 rand 依赖不值得。
    rng: u64,
}

impl Playback {
    /// `seed` 必须是奇数且非零，xorshift 从 0 出发会永远停在 0。
    pub fn new(mut snapshot: MiniSnapshot, seed: u64) -> Self {
        snapshot.normalize();
        Self {
            snapshot,
            failures: 0,
            notice: None,
            scrub: None,
            pending_seek: None,
            rng: seed | 1,
        }
    }

    pub fn snapshot(&self) -> &MiniSnapshot {
        &self.snapshot
    }

    /// 交回完整模式之前要改 `saved_at` 这类字段，给一个可变入口。
    /// 不要拿它去改 index/position——那些有对应的方法，绕过去会漏掉配套的状态重置。
    pub fn snapshot_mut(&mut self) -> &mut MiniSnapshot {
        &mut self.snapshot
    }

    pub fn index(&self) -> i64 {
        self.snapshot.index
    }

    pub fn count(&self) -> i64 {
        self.snapshot.tracks.len() as i64
    }

    pub fn notice(&self) -> Option<&'static str> {
        self.notice
    }

    pub fn volume(&self) -> f64 {
        self.snapshot.volume
    }

    pub fn muted(&self) -> bool {
        self.snapshot.muted
    }

    pub fn loop_mode(&self) -> u8 {
        self.snapshot.loop_mode
    }

    /// 拖动进度条时显示手指的位置而不是引擎的位置，不然拖起来会来回跳。
    pub fn display_position(&self) -> f64 {
        self.scrub.unwrap_or(self.snapshot.position)
    }

    pub fn scrub(&self) -> Option<f64> {
        self.scrub
    }

    pub fn set_scrub(&mut self, position: Option<f64>) {
        self.scrub = position;
    }

    /// 引擎报的位置。拖动进度条期间不要调用，否则显示会和手指打架。
    pub fn set_position(&mut self, position: f64) {
        self.snapshot.position = position;
    }

    /// 开始播放当前曲目。`resume` 表示这是刚从完整模式接管，要把进度接回去。
    pub fn begin_current(&mut self, resume: bool) -> Vec<Action> {
        self.scrub = None;
        self.pending_seek = None;

        let mut actions = vec![
            Action::SetVolume(self.snapshot.volume),
            Action::SetMuted(self.snapshot.muted),
        ];

        if self.snapshot.current().is_none() {
            self.notice = Some("播放列表是空的，回到完整模式选歌");
            actions.push(Action::Halt("播放列表是空的，回到完整模式选歌"));
            return actions;
        }

        // 恢复进度不能马上 seek：源还在打开中，这时候的 seek 会被丢掉，
        // 所以先记下来，等 take_pending_seek 看到源开好了再补。
        if resume && self.snapshot.position > RESUME_THRESHOLD {
            self.pending_seek = Some((self.snapshot.position, 0));
        }

        actions.push(Action::Load {
            index: self.snapshot.index,
        });
        actions.push(Action::Play);
        actions
    }

    /// 装载成功。清掉上一次的失败提示。
    pub fn on_load_ok(&mut self) {
        self.notice = None;
    }

    /// 装载失败：解析不出地址，或者引擎打不开。
    ///
    /// 一首打不开就往后跳，连挂三首就停手——再往后跳也是同样的结果，
    /// 不如告诉用户回完整模式重新解析。
    pub fn on_load_failed(&mut self) -> Vec<Action> {
        self.failures += 1;
        if self.failures >= MAX_FAILURES || self.snapshot.tracks.len() <= 1 {
            self.notice = Some("无可播放的曲目，回到完整模式重新解析");
            return vec![
                Action::Pause,
                Action::Halt("无可播放的曲目，回到完整模式重新解析"),
            ];
        }
        self.step(1, false)
    }

    /// 换曲。`user` 为真表示这是用户点的，会清掉失败计数，也不走随机。
    pub fn step(&mut self, delta: i64, user: bool) -> Vec<Action> {
        let count = self.count();
        if count == 0 {
            return Vec::new();
        }
        if user {
            self.failures = 0;
        }
        self.snapshot.index = if self.snapshot.loop_mode == LOOP_SHUFFLE && !user && delta > 0 {
            self.random_index(count)
        } else {
            (self.snapshot.index + delta).rem_euclid(count)
        };
        self.snapshot.position = 0.0;
        self.begin_current(false)
    }

    /// 跳到指定下标。用户点队列里的某一行。
    pub fn jump_to(&mut self, index: i64) -> Vec<Action> {
        let count = self.count();
        if count == 0 || index < 0 || index >= count {
            return Vec::new();
        }
        self.failures = 0;
        self.snapshot.index = index;
        self.snapshot.position = 0.0;
        self.begin_current(false)
    }

    /// 一首播完了。单曲循环重新装载同一首，否则往后一首。
    pub fn on_ended(&mut self) -> Vec<Action> {
        self.failures = 0;
        if self.snapshot.loop_mode == LOOP_ONE {
            // 不能只发 Seek(0) + Play。rodio 的队列播完就空了，对着空队列 seek 什么都
            // 不会发生，单曲循环会静悄悄地停住。必须重新 Load 同一首。
            //
            // 换后端之前用的是 WinRT MediaPlayer，源播完还挂在那儿，seek 回 0 是有效的，
            // 所以这个写法一直没暴露问题。
            self.snapshot.position = 0.0;
            return self.begin_current(false);
        }
        self.step(1, false)
    }

    /// 点播放/暂停。上一轮失败停下之后，再点一次当作重试。
    pub fn toggle_play(&mut self, is_playing: bool) -> Vec<Action> {
        if self.notice.is_some() {
            self.failures = 0;
            self.notice = None;
            return self.begin_current(false);
        }
        if is_playing {
            vec![Action::Pause]
        } else {
            vec![Action::Play]
        }
    }

    /// 循环模式轮换：顺序 → 单曲 → 随机 → 顺序。
    pub fn cycle_loop_mode(&mut self) -> u8 {
        self.snapshot.loop_mode = match self.snapshot.loop_mode {
            LOOP_SEQUENCE => LOOP_ONE,
            LOOP_ONE => LOOP_SHUFFLE,
            _ => LOOP_SEQUENCE,
        };
        self.snapshot.loop_mode
    }

    /// 补上被丢掉的恢复 seek。返回 Some 表示这一帧该 seek 到哪里。
    ///
    /// 一秒还没等到源打开就放弃，从头播总比不播好。
    pub fn take_pending_seek(&mut self, busy: bool, duration: f64) -> Option<f64> {
        let (target, tries) = self.pending_seek?;
        if !busy && duration > 0.0 {
            self.pending_seek = None;
            return Some(target);
        }
        if tries >= PENDING_SEEK_TRIES {
            self.pending_seek = None;
        } else {
            self.pending_seek = Some((target, tries + 1));
        }
        None
    }

    /// 只给测试用的观察点：外面不需要知道有没有待补的 seek，
    /// 但测试要能验证"换曲会丢掉上一首的 seek"这类行为。
    #[cfg(test)]
    pub fn has_pending_seek(&self) -> bool {
        self.pending_seek.is_some()
    }

    /// 拖音量条。手动拖的意思就是"我要听"，顺手解除静音。
    pub fn set_volume_from_ratio(&mut self, ratio: f64) -> Vec<Action> {
        self.snapshot.volume = ratio.clamp(0.0, 1.0);
        self.snapshot.muted = false;
        vec![
            Action::SetMuted(false),
            Action::SetVolume(self.snapshot.volume),
        ]
    }

    /// 滚轮调音量。不动静音状态——滚一下就解除静音会很意外。
    pub fn adjust_volume(&mut self, delta: f64) -> Vec<Action> {
        self.snapshot.volume = (self.snapshot.volume + delta).clamp(0.0, 1.0);
        vec![Action::SetVolume(self.snapshot.volume)]
    }

    pub fn toggle_muted(&mut self) -> Vec<Action> {
        self.snapshot.muted = !self.snapshot.muted;
        vec![Action::SetMuted(self.snapshot.muted)]
    }

    /// xorshift64。为一个洗牌拉一个 rand 依赖不值当。
    fn random_index(&mut self, count: i64) -> i64 {
        if count <= 1 {
            return 0;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        let mut pick = (self.rng % count as u64) as i64;
        // 随机播放至少得换一首，原地重放会被当成卡住了。
        if pick == self.snapshot.index {
            pick = (pick + 1) % count;
        }
        pick
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::MiniTrack;
    use super::*;

    const SEED: u64 = 0x9E3779B97F4A7C15;

    fn snapshot(count: usize) -> MiniSnapshot {
        MiniSnapshot {
            tracks: (0..count)
                .map(|i| MiniTrack {
                    id: i.to_string(),
                    title: format!("t{i}"),
                    source: "kuwo".to_string(),
                    local_path: Some(format!("/tmp/{i}.mp3")),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn playback(count: usize) -> Playback {
        Playback::new(snapshot(count), SEED)
    }

    fn loaded_index(actions: &[Action]) -> Option<i64> {
        actions.iter().find_map(|action| match action {
            Action::Load { index } => Some(*index),
            _ => None,
        })
    }

    #[test]
    fn sequence_mode_wraps_at_both_ends() {
        let mut p = playback(3);
        assert_eq!(p.index(), 0);
        p.step(1, true);
        assert_eq!(p.index(), 1);
        p.step(1, true);
        assert_eq!(p.index(), 2);
        p.step(1, true);
        assert_eq!(p.index(), 0, "末尾往后要绕到开头");
        p.step(-1, true);
        assert_eq!(p.index(), 2, "开头往前要绕到末尾");
    }

    #[test]
    fn single_track_queue_stays_on_itself() {
        let mut p = playback(1);
        p.step(1, true);
        assert_eq!(p.index(), 0);
        p.step(-1, true);
        assert_eq!(p.index(), 0);
    }

    #[test]
    fn empty_queue_produces_no_actions_and_halts() {
        let mut p = playback(0);
        assert!(p.step(1, true).is_empty(), "空队列换曲什么都不该发生");
        let actions = p.begin_current(false);
        assert!(actions.contains(&Action::Halt("播放列表是空的，回到完整模式选歌")));
        assert!(loaded_index(&actions).is_none());
        assert!(p.notice().is_some());
    }

    #[test]
    fn loop_one_reloads_instead_of_advancing() {
        let mut p = playback(3);
        p.snapshot.loop_mode = LOOP_ONE;
        let actions = p.on_ended();
        // 必须是重新 Load，不能是 Seek(0)：rodio 播完队列就空了，对空队列 seek 没有任何
        // 效果，单曲循环会静悄悄地停住。
        assert_eq!(loaded_index(&actions), Some(0), "单曲循环要重新装载同一首");
        assert!(actions.contains(&Action::Play));
        assert_eq!(p.index(), 0, "单曲循环不该换曲");
        assert_eq!(p.snapshot.position, 0.0, "重播要从头开始");
    }

    #[test]
    fn sequence_mode_advances_when_a_track_ends() {
        let mut p = playback(3);
        p.on_ended();
        assert_eq!(p.index(), 1);
    }

    #[test]
    fn shuffle_only_kicks_in_for_automatic_forward_steps() {
        let mut p = playback(10);
        p.snapshot.loop_mode = LOOP_SHUFFLE;

        // 用户点下一首：仍然是顺序的，不该被随机打乱
        p.step(1, true);
        assert_eq!(p.index(), 1);
        // 用户点上一首：同样顺序
        p.step(-1, true);
        assert_eq!(p.index(), 0);
    }

    #[test]
    fn shuffle_is_deterministic_for_a_given_seed() {
        let sequence = |seed: u64| {
            let mut p = Playback::new(snapshot(10), seed);
            p.snapshot.loop_mode = LOOP_SHUFFLE;
            (0..8)
                .map(|_| {
                    p.step(1, false);
                    p.index()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(sequence(SEED), sequence(SEED), "同一个种子必须复现同一串");
        assert_ne!(
            sequence(SEED),
            sequence(0x1234_5678_9ABC_DEF0),
            "不同种子不该给出同一串"
        );
    }

    #[test]
    fn shuffle_never_replays_the_same_track() {
        let mut p = playback(5);
        p.snapshot.loop_mode = LOOP_SHUFFLE;
        for _ in 0..200 {
            let before = p.index();
            p.step(1, false);
            assert_ne!(p.index(), before, "原地重放会被当成卡住了");
        }
    }

    #[test]
    fn shuffle_on_a_single_track_returns_zero() {
        let mut p = playback(1);
        p.snapshot.loop_mode = LOOP_SHUFFLE;
        p.step(1, false);
        assert_eq!(p.index(), 0);
    }

    #[test]
    fn three_consecutive_failures_halt_instead_of_skipping_forever() {
        let mut p = playback(10);
        // 前两次失败往后跳
        let first = p.on_load_failed();
        assert!(loaded_index(&first).is_some(), "第一次失败应当继续往后试");
        let second = p.on_load_failed();
        assert!(loaded_index(&second).is_some(), "第二次失败应当继续往后试");
        // 第三次停手
        let third = p.on_load_failed();
        assert!(
            third.contains(&Action::Halt("无可播放的曲目，回到完整模式重新解析")),
            "第三次必须停手，否则会把整个队列刷一遍"
        );
        assert!(third.contains(&Action::Pause));
        assert!(loaded_index(&third).is_none());
        assert!(p.notice().is_some());
    }

    #[test]
    fn a_single_track_queue_halts_on_the_first_failure() {
        let mut p = playback(1);
        let actions = p.on_load_failed();
        assert!(actions.contains(&Action::Halt("无可播放的曲目，回到完整模式重新解析")));
        assert!(loaded_index(&actions).is_none(), "只有一首，没有下一首可试");
    }

    #[test]
    fn a_successful_load_resets_the_failure_counter() {
        let mut p = playback(10);
        p.on_load_failed();
        p.on_load_failed();
        assert_eq!(p.failures, 2);
        p.on_load_ok();
        assert!(p.notice().is_none());
        // on_load_ok 只清提示；计数由用户操作或播完一首来清
        p.step(1, true);
        assert_eq!(p.failures, 0, "用户点换曲要清掉失败计数");

        p.on_load_failed();
        assert_eq!(p.failures, 1);
        p.on_ended();
        assert_eq!(p.failures, 0, "正常播完一首也要清掉失败计数");
    }

    #[test]
    fn retrying_after_a_halt_clears_the_notice_and_reloads() {
        let mut p = playback(1);
        p.on_load_failed();
        assert!(p.notice().is_some());
        // 停下之后再点播放当作重试
        let actions = p.toggle_play(false);
        assert!(p.notice().is_none());
        assert_eq!(p.failures, 0);
        assert_eq!(loaded_index(&actions), Some(0));
    }

    #[test]
    fn toggle_play_follows_the_engine_when_nothing_is_wrong() {
        let mut p = playback(3);
        assert_eq!(p.toggle_play(true), vec![Action::Pause]);
        assert_eq!(p.toggle_play(false), vec![Action::Play]);
    }

    #[test]
    fn resume_defers_the_seek_until_the_source_is_open() {
        let mut snap = snapshot(3);
        snap.position = 42.0;
        let mut p = Playback::new(snap, SEED);

        p.begin_current(true);
        assert!(p.has_pending_seek());
        // 源还在打开：不给 seek，但要留着
        assert_eq!(p.take_pending_seek(true, 0.0), None);
        assert!(p.has_pending_seek());
        // 源开好了：补上
        assert_eq!(p.take_pending_seek(false, 180.0), Some(42.0));
        assert!(!p.has_pending_seek());
    }

    #[test]
    fn a_source_that_never_opens_gives_up_after_a_second() {
        let mut snap = snapshot(3);
        snap.position = 42.0;
        let mut p = Playback::new(snap, SEED);
        p.begin_current(true);

        for _ in 0..PENDING_SEEK_TRIES {
            assert_eq!(p.take_pending_seek(true, 0.0), None);
        }
        assert_eq!(p.take_pending_seek(true, 0.0), None);
        assert!(!p.has_pending_seek(), "等不到就放弃，从头播总比不播好");
    }

    #[test]
    fn a_tiny_resume_position_is_not_worth_a_seek() {
        let mut snap = snapshot(3);
        snap.position = RESUME_THRESHOLD;
        let mut p = Playback::new(snap, SEED);
        p.begin_current(true);
        assert!(!p.has_pending_seek());
    }

    #[test]
    fn changing_track_drops_a_stale_pending_seek() {
        let mut snap = snapshot(3);
        snap.position = 42.0;
        let mut p = Playback::new(snap, SEED);
        p.begin_current(true);
        assert!(p.has_pending_seek());
        p.step(1, true);
        assert!(!p.has_pending_seek(), "换曲之后上一首的进度不该再被 seek 上去");
    }

    #[test]
    fn scrub_wins_over_the_engine_position_while_dragging() {
        let mut p = playback(3);
        p.set_position(30.0);
        assert_eq!(p.display_position(), 30.0);
        p.set_scrub(Some(90.0));
        assert_eq!(p.display_position(), 90.0, "拖动时显示手指的位置");
        p.set_scrub(None);
        assert_eq!(p.display_position(), 30.0);
    }

    #[test]
    fn dragging_the_volume_slider_also_unmutes() {
        let mut p = playback(3);
        p.snapshot.muted = true;
        let actions = p.set_volume_from_ratio(0.5);
        assert_eq!(p.volume(), 0.5);
        assert!(!p.muted(), "手动拖音量的意思就是我要听");
        assert!(actions.contains(&Action::SetMuted(false)));
        assert!(actions.contains(&Action::SetVolume(0.5)));
    }

    #[test]
    fn volume_is_clamped_from_both_directions() {
        let mut p = playback(3);
        p.set_volume_from_ratio(5.0);
        assert_eq!(p.volume(), 1.0);
        p.set_volume_from_ratio(-5.0);
        assert_eq!(p.volume(), 0.0);

        p.adjust_volume(0.3);
        assert!((p.volume() - 0.3).abs() < 1e-9);
        p.adjust_volume(10.0);
        assert_eq!(p.volume(), 1.0);
        p.adjust_volume(-10.0);
        assert_eq!(p.volume(), 0.0);
    }

    #[test]
    fn scrolling_the_volume_does_not_unmute() {
        let mut p = playback(3);
        p.snapshot.muted = true;
        p.adjust_volume(0.1);
        assert!(p.muted(), "滚一下就解除静音会很意外");
    }

    #[test]
    fn loop_mode_cycles_through_three_states_and_back() {
        let mut p = playback(3);
        assert_eq!(p.loop_mode(), LOOP_SEQUENCE);
        assert_eq!(p.cycle_loop_mode(), LOOP_ONE);
        assert_eq!(p.cycle_loop_mode(), LOOP_SHUFFLE);
        assert_eq!(p.cycle_loop_mode(), LOOP_SEQUENCE);
    }

    #[test]
    fn jump_to_ignores_out_of_range_indices() {
        let mut p = playback(3);
        assert!(p.jump_to(-1).is_empty());
        assert!(p.jump_to(3).is_empty());
        assert_eq!(p.index(), 0, "越界的点击不该改变当前曲目");
        assert_eq!(loaded_index(&p.jump_to(2)), Some(2));
        assert_eq!(p.index(), 2);
    }

    #[test]
    fn begin_current_always_pushes_volume_and_mute_first() {
        let mut p = playback(3);
        let actions = p.begin_current(false);
        assert_eq!(actions[0], Action::SetVolume(p.volume()));
        assert_eq!(actions[1], Action::SetMuted(p.muted()));
    }
}

