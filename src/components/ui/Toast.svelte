<script lang="ts">
  import { toasts, removeToast } from "../../lib/stores/toast";
  import { fly, fade } from "svelte/transition";
  import { quintOut } from "svelte/easing";
</script>

<div class="toast-stack">
  {#each $toasts as t (t.id)}
    <div
      class="toast toast-{t.kind}"
      role="status"
      aria-live="polite"
      in:fly={{ y: -14, duration: 320, easing: quintOut }}
      out:fade={{ duration: 220 }}
    >
      <div class="dot"></div>
      <span class="msg">{t.message}</span>
      <button type="button" class="dismiss" aria-label="关闭" onclick={() => removeToast(t.id)}>&times;</button>
    </div>
  {/each}
</div>

<style>
  .toast-stack {
    position: fixed;
    top: 76px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 9999;
    display: flex;
    flex-direction: column;
    gap: 8px;
    pointer-events: none;
  }

  .toast {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 220px;
    max-width: 480px;
    padding: 10px 18px;
    border-radius: 999px;
    background: var(--glass-bg);
    backdrop-filter: var(--glass-blur);
    -webkit-backdrop-filter: var(--glass-blur);
    border: 1px solid var(--glass-border);
    box-shadow: var(--shadow);
    color: var(--text-default-color);
    font-size: 13px;
    font-weight: 500;
    pointer-events: auto;
    user-select: none;
  }

  .dismiss {
    all: unset;
    cursor: pointer;
    font-size: 16px;
    line-height: 1;
    opacity: 0.5;
    padding: 2px 6px;
    border-radius: 50%;
    transition: opacity 0.15s, background 0.15s;
  }
  .dismiss:hover { opacity: 1; background: rgba(255,255,255,0.1); }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--theme-color);
    flex-shrink: 0;
    animation: glow 1.6s ease-in-out infinite;
  }

  .toast-success .dot { background: #34c759; --theme-color-glow: rgba(52,199,89,0.5); }
  .toast-warn .dot    { background: #ff9500; --theme-color-glow: rgba(255,149,0,0.5); }
  .toast-error .dot   { background: #ff3b30; --theme-color-glow: rgba(255,59,48,0.5); }

  .msg { flex: 1; }
</style>
