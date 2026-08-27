<script lang="ts">
  /**
   * 最小化 / 最大化 / 关闭。
   *
   * 抽成组件是因为要在两处渲染：常态在标题栏里，底部播放器展开后标题栏被
   * `.footer.expanded`（z-index 320）整块盖住，那时改由播放器内部再渲染一份悬浮版。
   * 两处共用同一套按钮与样式，避免行为漂移。
   *
   * 注意：`decorations: false` + `transparent: true`，这三个按钮是唯一的窗口控件，
   * 任何时候都不能变成点不到的状态。
   */
  import { getTauriRuntimeDiagnostics, windowMinimize, windowMaximize, windowClose } from "../../lib/tauri";

  function runWindowCommand(name: string, command: () => Promise<unknown>) {
    command().catch((error) => {
      console.error(`[WindowControls] ${name} failed`, {
        error,
        tauri: getTauriRuntimeDiagnostics(),
      });
    });
  }
</script>

<div class="window-control">
  <button type="button" class="wc-btn" onclick={() => runWindowCommand("minimize", windowMinimize)} aria-label="最小化">
    <svg width="18" height="18" viewBox="0 0 24 24">
      <line x1="5" y1="12" x2="19" y2="12"/>
    </svg>
  </button>
  <button type="button" class="wc-btn" onclick={() => runWindowCommand("maximize", windowMaximize)} aria-label="最大化">
    <svg width="18" height="18" viewBox="0 0 24 24">
      <rect x="3" y="3" width="18" height="18" rx="2"/>
    </svg>
  </button>
  <button type="button" class="wc-btn close-btn" onclick={() => runWindowCommand("close", windowClose)} aria-label="关闭">
    <svg width="18" height="18" viewBox="0 0 24 24">
      <path d="M18 6L6 18M6 6l12 12"/>
    </svg>
  </button>
</div>

<style>
  .window-control {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .wc-btn {
    all: unset;
    margin-left: 4px;
    padding: 6px;
    box-sizing: content-box;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    -webkit-app-region: no-drag;
    transition: background 0.2s;
  }

  .wc-btn svg {
    stroke: var(--player-icon-color);
    transition: stroke var(--dur-fast) var(--ease-soft);
  }

  .wc-btn:hover {
    background: var(--songlist-hover-background-color);
  }
  .wc-btn:hover svg {
    stroke: var(--text-default-color);
  }
  .wc-btn:active { opacity: 0.8; }
  .close-btn:hover {
    background: #ff4444;
  }
  .close-btn:hover svg {
    stroke: #fff;
  }

  /* 窄窗收紧按钮，和标题栏其余元素一起变小（原先写在 TitleBar 里，随按钮一并搬来） */
  @media (max-width: 720px) {
    .wc-btn {
      margin-left: 0;
      padding: 5px;
    }

    .wc-btn svg {
      width: 16px;
      height: 16px;
    }
  }

  @media (max-width: 440px) {
    .wc-btn {
      padding: 4px;
    }
  }
</style>
