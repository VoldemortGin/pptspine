//! 解析期预算的线程局部状态 + 单值长度上限。
//!
//! 一个部件的解析由 `Package::budgeted` 包起来:进入时 [`begin`] 给出本部件可用的节点数 /
//! 模型字节数,退出时 [`end`] 取回用量。walker 里每建模一个节点调用 [`take_item`](带该节点的
//! 结构体大小),每产生一个字符串经 [`fit_string`](`attr_string` / `read_text` 已内置)按实际
//! 字节扣减——walker 函数不带上下文,与嵌套计数同理用线程局部状态。不在任何 `begin` 内时
//! 预算视为无限(只做单值长度截断)。
//!
//! 单值上限只截"单个字符串":属性值、文本节点、被多处引用的短标签。它们在模型里可能被引用
//! 很多次(批注作者、超链接目标、图表类别名),上限让"一个值 × 引用次数"有界。

use std::cell::Cell;

/// 单个属性值的长度上限(字节)。现实属性值(替代文本、公式、字体名)至多几 KB。
pub(crate) const MAX_ATTR_BYTES: usize = 64 * 1024;
/// 单个文本节点(`a:t` / `m:t` / `c:v` / 批注正文 …)的长度上限(字节)。一页幻灯片的一个
/// run 现实中至多几 KB;1 MiB 宽松到不会误伤,又让单个节点有界。
pub(crate) const MAX_TEXT_BYTES: usize = 1024 * 1024;
/// 被多处引用 / 逐 frame 展开的短标签(批注作者名 / 缩写、图表类别名 / 系列名 / 标题)的长度
/// 上限(字节)。现实中这些标签几十字节。
pub(crate) const MAX_LABEL_BYTES: usize = 4 * 1024;
/// 超链接目标 URL 的长度上限(字节)。浏览器常见上限 2–8 KB,16 KiB 宽松。
pub(crate) const MAX_URL_BYTES: usize = 16 * 1024;

/// 一次 [`begin`]..[`end`] 之间的用量。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Usage {
    /// 实际建模的节点数。
    pub items_used: usize,
    /// 因节点 / 字节预算耗尽而丢弃的节点数(含被整体跳过的容器的后代)。
    pub items_dropped: usize,
    /// 实际扣减的模型字节数(结构体大小 + 字符串字节)。
    pub bytes_used: usize,
    /// 被截短的字符串值个数(单值上限或字节预算耗尽)。
    pub values_truncated: usize,
}

#[derive(Clone, Copy)]
struct State {
    items_left: usize,
    bytes_left: usize,
    usage: Usage,
}

const UNLIMITED: State = State {
    items_left: usize::MAX,
    bytes_left: usize::MAX,
    usage: Usage {
        items_used: 0,
        items_dropped: 0,
        bytes_used: 0,
        values_truncated: 0,
    },
};

thread_local! {
    static STATE: Cell<State> = const { Cell::new(UNLIMITED) };
}

/// 保存的外层状态([`begin`] 返回,交给 [`end`] 恢复;支持嵌套)。
pub(crate) struct Saved(State);

/// 开始一段受限解析:最多 `items` 个节点、`bytes` 模型字节。
pub(crate) fn begin(items: usize, bytes: usize) -> Saved {
    Saved(STATE.with(|s| {
        s.replace(State {
            items_left: items,
            bytes_left: bytes,
            usage: Usage::default(),
        })
    }))
}

/// 结束受限解析,恢复外层状态并返回本段用量(外层的字节额度同步扣减)。
pub(crate) fn end(saved: Saved) -> Usage {
    let inner = STATE.with(|s| s.replace(saved.0));
    if inner.usage.bytes_used > 0 || inner.usage.items_used > 0 {
        STATE.with(|s| {
            let mut o = s.get();
            o.bytes_left = o.bytes_left.saturating_sub(inner.usage.bytes_used);
            o.items_left = o.items_left.saturating_sub(inner.usage.items_used);
            o.usage.bytes_used = o.usage.bytes_used.saturating_add(inner.usage.bytes_used);
            o.usage.items_used = o.usage.items_used.saturating_add(inner.usage.items_used);
            s.set(o);
        });
    }
    inner.usage
}

fn update<T>(f: impl FnOnce(&mut State) -> T) -> T {
    STATE.with(|s| {
        let mut st = s.get();
        let out = f(&mut st);
        s.set(st);
        out
    })
}

/// 申请建模一个节点(结构体约 `bytes` 字节):节点或字节额度用尽返回 `false`(记一次丢弃,
/// 调用方应整体跳过该元素——见 [`skip_dropped`])。
pub(crate) fn take_item(bytes: usize) -> bool {
    update(|st| {
        if st.items_left == 0 || st.bytes_left < bytes {
            st.usage.items_dropped = st.usage.items_dropped.saturating_add(1);
            false
        } else {
            st.items_left -= 1;
            st.bytes_left -= bytes;
            st.usage.items_used += 1;
            st.usage.bytes_used = st.usage.bytes_used.saturating_add(bytes);
            true
        }
    })
}

/// 申请 `bytes` 模型字节(不占节点数,如形状本体 / 克隆副本):不够返回 `false`(不扣)。
pub(crate) fn take_bytes(bytes: usize) -> bool {
    update(|st| {
        if st.bytes_left < bytes {
            false
        } else {
            st.bytes_left -= bytes;
            st.usage.bytes_used = st.usage.bytes_used.saturating_add(bytes);
            true
        }
    })
}

/// 记 `n` 个被丢弃的节点(被整体跳过的容器的后代)。
pub(crate) fn note_dropped(n: usize) {
    if n > 0 {
        update(|st| st.usage.items_dropped = st.usage.items_dropped.saturating_add(n));
    }
}

/// 记一个被截短的值。
pub(crate) fn note_truncated() {
    update(|st| st.usage.values_truncated = st.usage.values_truncated.saturating_add(1));
}

/// 当前剩余的模型字节额度。
pub(crate) fn bytes_left() -> usize {
    STATE.with(|s| s.get().bytes_left)
}

/// 把 `s` 截到不超过 `cap` 字节与剩余字节额度(按字符边界),再按截后长度扣减;发生截短
/// 记一次 [`Usage::values_truncated`]。
pub(crate) fn fit_string(s: &mut String, cap: usize) {
    update(|st| {
        let max = cap.min(st.bytes_left);
        let cut = s.len() > max;
        if cut {
            let mut end = max;
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            s.truncate(end);
        }
        st.bytes_left -= s.len();
        st.usage.bytes_used = st.usage.bytes_used.saturating_add(s.len());
        if cut {
            st.usage.values_truncated = st.usage.values_truncated.saturating_add(1);
        }
    });
}

/// 把字符串截到最多 `max_chars` 个字符(公式定界符等"规范上是单字符"的值);截短记一次。
pub(crate) fn fit_chars(s: &mut String, max_chars: usize) {
    if let Some((i, _)) = s.char_indices().nth(max_chars) {
        s.truncate(i);
        note_truncated();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_are_capped_on_char_boundaries_and_charged() {
        let saved = begin(10, 100);
        let mut s = "é".repeat(10); // 20 字节
        fit_string(&mut s, 7);
        assert_eq!(s, "ééé");
        let mut t = "x".repeat(200);
        fit_string(&mut t, MAX_TEXT_BYTES); // 只剩 94 字节额度
        assert_eq!(t.len(), 94);
        let u = end(saved);
        assert_eq!((u.bytes_used, u.values_truncated), (100, 2));
    }

    #[test]
    fn nested_budgets_charge_the_outer_one() {
        let outer = begin(100, 1_000);
        let inner = begin(10, 50);
        assert!(take_item(40));
        assert!(!take_item(40));
        let iu = end(inner);
        assert_eq!((iu.items_used, iu.items_dropped, iu.bytes_used), (1, 1, 40));
        assert_eq!(bytes_left(), 960);
        let ou = end(outer);
        assert_eq!(ou.items_used, 1);
    }

    #[test]
    fn unlimited_outside_begin_still_caps_single_values() {
        let mut s = "a".repeat(MAX_ATTR_BYTES + 10);
        fit_string(&mut s, MAX_ATTR_BYTES);
        assert_eq!(s.len(), MAX_ATTR_BYTES);
        let mut c = "[[[".to_string();
        fit_chars(&mut c, 2);
        assert_eq!(c, "[[");
    }
}
