//! Built-in board shape catalog, technical icon catalog, palette, and interaction constants.
//! Mirrors `clients/mica_flutter/lib/board/board_models.dart` and `board_icons.dart` 1:1.

pub const NODE_PADDING_H: f64 = 10.0;
pub const NODE_PADDING_V: f64 = 8.0;
pub const SNAP_THRESHOLD: f64 = 6.0;
pub const SNAP_THRESHOLD_MIN: f64 = 2.5;
pub const MAX_HISTORY: usize = 100;
pub const MIN_NODE_W: f64 = 56.0;
pub const MIN_NODE_H: f64 = 34.0;
pub const ZOOM_MIN: f64 = 0.2;
pub const ZOOM_MAX: f64 = 3.0;
pub const FIT_ZOOM_MIN: f64 = 0.3;
pub const FIT_ZOOM_MAX: f64 = 1.35;
pub const DEFAULT_PAN: [f64; 2] = [80.0, 60.0];
pub const DEFAULT_VIEWPORT: [f64; 2] = [1280.0, 800.0];
pub const PEN_MIN_DIST: f64 = 3.0;
pub const DRAG_MOVE_THRESHOLD: f64 = 0.5;
pub const DOUBLE_CLICK_MS: u64 = 420;
pub const FRAME_TITLE_BAR_H: f64 = 30.0;
pub const FRAME_OUTER_INFLATE: f64 = 6.0;
pub const FRAME_INNER_DEFLATE: f64 = 14.0;
pub const COLLAPSE_BADGE_HIT_RADIUS: f64 = 11.0;
pub const ANCHOR_PORT_HIT_RADIUS: f64 = 9.0;
pub const RESIZE_HANDLE_HIT_RADIUS: f64 = 8.0;
pub const EDGE_HIT_TOLERANCE: f64 = 8.0;
pub const DEFAULT_PASTE_OFFSET: [f64; 2] = [28.0, 28.0];

pub const MIND_BRANCH_PALETTE: &[(&str, &str)] = &[
  ("#E8F3FF", "#3370FF"),
  ("#EDF7ED", "#2DA44E"),
  ("#FFF7E6", "#D9822B"),
  ("#F5F0FF", "#8250DF"),
  ("#FFF0F0", "#CF222E"),
  ("#E6F7F7", "#0E8A8A"),
];

#[derive(Debug, Clone, Copy)]
pub struct ShapeMeta {
  pub kind: &'static str,
  pub label: &'static str,
  pub category: &'static str,
  pub default_w: f64,
  pub default_h: f64,
  pub default_fill: &'static str,
  pub default_stroke: &'static str,
}

pub const BOARD_SHAPES: &[ShapeMeta] = &[
  ShapeMeta {
    kind: "rounded",
    label: "圆角矩形",
    category: "basic",
    default_w: 168.0,
    default_h: 58.0,
    default_fill: "#E8F3FF",
    default_stroke: "#3370FF",
  },
  ShapeMeta {
    kind: "rect",
    label: "矩形",
    category: "basic",
    default_w: 160.0,
    default_h: 58.0,
    default_fill: "#FFFFFF",
    default_stroke: "#646A73",
  },
  ShapeMeta {
    kind: "ellipse",
    label: "椭圆",
    category: "basic",
    default_w: 144.0,
    default_h: 80.0,
    default_fill: "#EDF7ED",
    default_stroke: "#2DA44E",
  },
  ShapeMeta {
    kind: "triangle",
    label: "三角形",
    category: "basic",
    default_w: 132.0,
    default_h: 96.0,
    default_fill: "#FFF7E6",
    default_stroke: "#D9822B",
  },
  ShapeMeta {
    kind: "hexagon",
    label: "六边形",
    category: "basic",
    default_w: 164.0,
    default_h: 66.0,
    default_fill: "#F5F0FF",
    default_stroke: "#8250DF",
  },
  ShapeMeta {
    kind: "cloud",
    label: "云朵",
    category: "basic",
    default_w: 176.0,
    default_h: 96.0,
    default_fill: "#E8F3FF",
    default_stroke: "#3370FF",
  },
  ShapeMeta {
    kind: "pill",
    label: "开始 / 结束",
    category: "flowchart",
    default_w: 140.0,
    default_h: 46.0,
    default_fill: "#EDF7ED",
    default_stroke: "#2DA44E",
  },
  ShapeMeta {
    kind: "diamond",
    label: "判断条件",
    category: "flowchart",
    default_w: 156.0,
    default_h: 86.0,
    default_fill: "#FFF7E6",
    default_stroke: "#D9822B",
  },
  ShapeMeta {
    kind: "parallelogram",
    label: "输入 / 输出",
    category: "flowchart",
    default_w: 168.0,
    default_h: 58.0,
    default_fill: "#E6F7F7",
    default_stroke: "#0E8A8A",
  },
  ShapeMeta {
    kind: "cylinder",
    label: "数据库",
    category: "flowchart",
    default_w: 144.0,
    default_h: 76.0,
    default_fill: "#F5F0FF",
    default_stroke: "#8250DF",
  },
  ShapeMeta {
    kind: "document",
    label: "文档",
    category: "flowchart",
    default_w: 160.0,
    default_h: 72.0,
    default_fill: "#FFFFFF",
    default_stroke: "#3370FF",
  },
  ShapeMeta {
    kind: "sticky",
    label: "便签",
    category: "basic",
    default_w: 180.0,
    default_h: 112.0,
    default_fill: "#FFF7CC",
    default_stroke: "#E0B422",
  },
];

pub fn shape_meta(kind: &str) -> &'static ShapeMeta {
  BOARD_SHAPES
    .iter()
    .find(|s| s.kind == kind)
    .unwrap_or(&BOARD_SHAPES[0])
}

#[derive(Debug, Clone, Copy)]
pub struct TechIconMeta {
  pub id: &'static str,
  pub label: &'static str,
  pub group: &'static str,
  pub default_fill: &'static str,
  pub default_stroke: &'static str,
}

pub const BOARD_TECH_ICONS: &[TechIconMeta] = &[
  TechIconMeta {
    id: "server",
    label: "服务器",
    group: "计算",
    default_fill: "#E8F3FF",
    default_stroke: "#3370FF",
  },
  TechIconMeta {
    id: "container",
    label: "容器 / K8s",
    group: "计算",
    default_fill: "#E8F3FF",
    default_stroke: "#3370FF",
  },
  TechIconMeta {
    id: "api",
    label: "API 服务",
    group: "计算",
    default_fill: "#EDF7ED",
    default_stroke: "#2DA44E",
  },
  TechIconMeta {
    id: "worker",
    label: "后台任务",
    group: "计算",
    default_fill: "#EDF7ED",
    default_stroke: "#2DA44E",
  },
  TechIconMeta {
    id: "database",
    label: "数据库",
    group: "数据",
    default_fill: "#F5F0FF",
    default_stroke: "#8250DF",
  },
  TechIconMeta {
    id: "cache",
    label: "缓存 Redis",
    group: "数据",
    default_fill: "#FFF0F0",
    default_stroke: "#CF222E",
  },
  TechIconMeta {
    id: "queue",
    label: "消息队列",
    group: "数据",
    default_fill: "#FFF7E6",
    default_stroke: "#D9822B",
  },
  TechIconMeta {
    id: "storage",
    label: "对象存储",
    group: "数据",
    default_fill: "#F5F0FF",
    default_stroke: "#8250DF",
  },
  TechIconMeta {
    id: "gateway",
    label: "网关",
    group: "网络",
    default_fill: "#E6F7F7",
    default_stroke: "#0E8A8A",
  },
  TechIconMeta {
    id: "lb",
    label: "负载均衡",
    group: "网络",
    default_fill: "#E6F7F7",
    default_stroke: "#0E8A8A",
  },
  TechIconMeta {
    id: "cdn",
    label: "CDN 节点",
    group: "网络",
    default_fill: "#E8F3FF",
    default_stroke: "#3370FF",
  },
  TechIconMeta {
    id: "cloud",
    label: "云服务",
    group: "网络",
    default_fill: "#E8F3FF",
    default_stroke: "#3370FF",
  },
  TechIconMeta {
    id: "browser",
    label: "Web 端",
    group: "终端",
    default_fill: "#FFFFFF",
    default_stroke: "#646A73",
  },
  TechIconMeta {
    id: "mobile",
    label: "移动端",
    group: "终端",
    default_fill: "#FFFFFF",
    default_stroke: "#646A73",
  },
  TechIconMeta {
    id: "user",
    label: "用户",
    group: "终端",
    default_fill: "#FFF7E6",
    default_stroke: "#D9822B",
  },
  TechIconMeta {
    id: "doc",
    label: "文档",
    group: "终端",
    default_fill: "#FFFFFF",
    default_stroke: "#3370FF",
  },
  TechIconMeta {
    id: "firewall",
    label: "防火墙",
    group: "安全与AI",
    default_fill: "#FFF0F0",
    default_stroke: "#CF222E",
  },
  TechIconMeta {
    id: "lock",
    label: "鉴权加密",
    group: "安全与AI",
    default_fill: "#FFF7E6",
    default_stroke: "#D9822B",
  },
  TechIconMeta {
    id: "monitor",
    label: "监控告警",
    group: "安全与AI",
    default_fill: "#EDF7ED",
    default_stroke: "#2DA44E",
  },
  TechIconMeta {
    id: "ai",
    label: "AI / LLM",
    group: "安全与AI",
    default_fill: "#F5F0FF",
    default_stroke: "#8250DF",
  },
];

pub fn tech_icon_meta(id: &str) -> &'static TechIconMeta {
  BOARD_TECH_ICONS
    .iter()
    .find(|m| m.id == id)
    .unwrap_or(&BOARD_TECH_ICONS[0])
}
