//! 端末 setup / teardown と描画サーフェス。
//! raw mode と alternate screen の復帰を RAII に寄せ、mouse capture は有効化しない。
use super::*;

/// raw mode + alternate screen の出入りを RAII で包む描画サーフェス。
/// Drop で必ず raw mode 解除・画面復帰・カーソル表示を行い、パニック経路でも
/// 端末が壊れた状態で残らないようにする。
/// mouse capture は意図的に有効化しない:端末側にドラッグ選択 / コピーを残すためで、
/// その代わりスクロールはキーボード(PageUp / PageDown)で行う(Phase 25.1 の判断)。
pub(super) struct TuiTerminal {
    pub(super) terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl TuiTerminal {
    /// raw mode を有効化し alternate screen に入って描画サーフェスを作る。
    /// 途中で失敗した場合は raw mode / 画面状態を巻き戻してから Err を返す。
    pub(super) fn enter() -> Result<Self, String> {
        enable_raw_mode().map_err(|error| format!("Could not enable raw mode: {error}"))?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(format!("Could not enter alternate screen: {error}"));
        }

        let backend = CrosstermBackend::new(stdout);
        match Terminal::new(backend) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                cleanup_terminal();
                Err(format!("Could not initialize terminal: {error}"))
            }
        }
    }

    /// 1フレーム描画する。クロージャに Frame を渡し、完了フラグは捨てる。
    pub(super) fn draw(&mut self, draw: impl FnOnce(&mut Frame<'_>)) -> io::Result<()> {
        self.terminal.draw(draw).map(|_| ())
    }

    /// 現在の端末サイズを原点 (0,0) の Rect として返す(レイアウト計算用)。
    pub(super) fn size(&self) -> io::Result<Rect> {
        self.terminal
            .size()
            .map(|size| Rect::new(0, 0, size.width, size.height))
    }
}

impl Drop for TuiTerminal {
    /// 端末を元の状態へ戻す後始末:raw mode 解除 → 画面復帰 → カーソル再表示。
    /// 失敗は無視する(これ以上の復帰手段が無く、エラー伝播もできない Drop のため)。
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen, Show);
        let _ = self.terminal.show_cursor();
    }
}

/// TuiTerminal を保持できない早期失敗 / パニック経路向けの最小復帰。
/// raw mode を解除し alternate screen を抜けてカーソルを戻す。
pub(super) fn cleanup_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}
