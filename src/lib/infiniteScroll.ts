// 可复用的「滑到底部自动加载」action。
// 用法：<div use:infiniteScroll={{ onLoadMore, disabled }}></div>
// 把它挂在列表末尾的哨兵元素上；哨兵进入滚动容器视口时触发 onLoadMore。
// root 默认取最近的可滚动祖先（本项目为 App.svelte 的 .browser），无需显式传入。

export type InfiniteScrollParams = {
  onLoadMore: () => void;
  // 为 true 时暂停触发（加载中 / 没有更多 / 未就绪）。
  disabled?: boolean;
  // 提前触发的距离，默认视口下方 320px 处就开始加载。
  rootMargin?: string;
};

export function infiniteScroll(node: HTMLElement, params: InfiniteScrollParams) {
  let current = params;

  const handler: IntersectionObserverCallback = (entries) => {
    for (const entry of entries) {
      if (entry.isIntersecting && !current.disabled) {
        current.onLoadMore();
      }
    }
  };

  const observer = new IntersectionObserver(handler, {
    root: null,
    rootMargin: params.rootMargin ?? "0px 0px 320px 0px",
    threshold: 0,
  });
  observer.observe(node);

  return {
    update(next: InfiniteScrollParams) {
      current = next;
    },
    destroy() {
      observer.disconnect();
    },
  };
}
