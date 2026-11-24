//! 360×420 curated bilingual Emoji Picker modal (1:1 parity with
//! `main:clients/mica_flutter/lib/ui/emoji_picker.dart`).
//!
//! Features:
//! - Header with `选择图标` + `移除图标` button (calls `on_pick(None)`).
//! - Auto-focused search box (`搜索表情…`) with bilingual Chinese + English keyword
//!   filtering across 7 curated categories (`表情`/`人物`/`自然`/`食物`/`物件`/`符号`/`旗帜`).
//! - Enter picks the first match (when query is non-empty); Esc or backdrop click closes.
//! - Scrollable 8-column grid of `38×38px` emoji cells (`3px` gap), highlighting `current_icon`.
//! - Calm empty state (`没有匹配的表情`).

use std::rc::Rc;

use nsh_base::{button, input};
use nsh_core::{SubmitBehavior, geometry::px, keymap::When, prelude::*};

use crate::{mica_icons, mica_theme::palette};

/// One curated emoji with lowercase English + Chinese search keywords.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmojiEntry {
  pub glyph: &'static str,
  pub keywords: &'static str,
}

const fn e(glyph: &'static str, keywords: &'static str) -> EmojiEntry {
  EmojiEntry { glyph, keywords }
}

const CELL_SIZE: f32 = 38.0;
const CELL_GAP: f32 = 3.0;
const GRID_COLUMNS: usize = 8;

/// All 7 curated emoji categories matching Mica Flutter 1:1.
pub const EMOJI_CATEGORIES: &[(&str, &[EmojiEntry])] = &[
  ("表情", SMILEYS),
  ("人物", PEOPLE),
  ("自然", NATURE),
  ("食物", FOOD),
  ("物件", OBJECTS),
  ("符号", SYMBOLS),
  ("旗帜", FLAGS),
];

/// Filters all 7 emoji categories by a trimmed, case-insensitive substring query
/// against both the glyph itself and its bilingual Chinese/English keywords.
pub fn filter_emoji_sections(query: &str) -> Vec<(&'static str, Vec<EmojiEntry>)> {
  let needle = query.trim().to_lowercase();
  let mut out = Vec::new();
  for &(label, rows) in EMOJI_CATEGORIES {
    let hits: Vec<EmojiEntry> = rows
      .iter()
      .copied()
      .filter(|row| needle.is_empty() || row.glyph == needle || row.keywords.contains(&needle))
      .collect();
    if !hits.is_empty() {
      out.push((label, hits));
    }
  }
  out
}

/// Returns the first matching emoji for Enter-to-pick (returns `None` when query is empty,
/// matching Flutter's rule that Enter on an empty search box does nothing).
pub fn first_matching_emoji(query: &str) -> Option<&'static str> {
  if query.trim().is_empty() {
    return None;
  }
  filter_emoji_sections(query)
    .first()
    .and_then(|(_, rows)| rows.first())
    .map(|entry| entry.glyph)
}

/// Renders the centered `360×420` Emoji Picker modal when `open` is `true`.
pub fn emoji_picker_modal(
  open: Signal<bool>,
  current_icon: Signal<Option<String>>,
  dark_mode: Signal<bool>,
  on_pick: Rc<dyn Fn(Option<String>)>,
  on_close: Rc<dyn Fn()>,
) -> impl IntoElement {
  let query = signal(String::new());

  // Reset search query on open and bind Esc to close regardless of focus.
  let close_esc = on_close.clone();
  effect(move || {
    if !open.get() {
      return;
    }
    query.set(String::new());
    let close_key = close_esc.clone();
    on_key_with(
      "escape",
      KeyOptions {
        when: When::Always,
        repeat: false,
        pass_through: true,
        ..KeyOptions::default()
      },
      move |_| {
        if open.get_untracked() {
          close_key();
        }
      },
    );
  });

  when(move || open.get(), {
    let pick_outer = on_pick.clone();
    let close_outer = on_close.clone();
    move || {
      let pick_remove = pick_outer.clone();
      let close_remove = close_outer.clone();
      let pick_submit = pick_outer.clone();
      let close_submit = close_outer.clone();
      let close_backdrop = close_outer.clone();
      let close_input_esc = close_outer.clone();
      let pick_grid = pick_outer.clone();
      let close_grid = close_outer.clone();

      view()
        .test_id("emoji-picker-backdrop")
        .absolute()
        .inset_0()
        .z_index(60)
        .items_center()
        .justify_center()
        .bg(move || palette(dark_mode.get()).backdrop)
        .child(
          button()
            .test_id("emoji-picker-dismiss")
            .accessibility_label("关闭图标选择器")
            .absolute()
            .inset_0()
            .on_press(move || close_backdrop()),
        )
        .child(
          view()
            .test_id("emoji-picker-modal")
            .w(px(360.0))
            .h(px(420.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .overflow_hidden()
            .z_index(61)
            // Header: `选择图标` + `移除图标`
            .child(
              view()
                .pl(px(14.0))
                .pr(px(10.0))
                .pt(px(12.0))
                .pb(px(10.0))
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                  text("选择图标")
                    .text_size(px(14.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || palette(dark_mode.get()).text_primary),
                )
                .child(
                  button()
                    .test_id("emoji-picker-remove")
                    .accessibility_label("移除图标")
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(move || palette(dark_mode.get()).border_normal)
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || {
                      pick_remove(None);
                      close_remove();
                    })
                    .child(
                      text("移除图标")
                        .text_size(px(12.0))
                        .text_color(move || palette(dark_mode.get()).text_muted),
                    ),
                ),
            )
            // Auto-focused search box (`搜索表情…`)
            .child(
              view().px(px(14.0)).pb(px(10.0)).child(
                view()
                  .h(px(34.0))
                  .px(px(10.0))
                  .flex_row()
                  .items_center()
                  .gap_2()
                  .rounded(px(8.0))
                  .border_1()
                  .border_color(move || palette(dark_mode.get()).border_normal)
                  .bg(move || palette(dark_mode.get()).surface_overlay)
                  .child(dynamic(move || {
                    mica_icons::search(palette(dark_mode.get()).text_faint_hex)
                  }))
                  .child(
                    input()
                      .test_id("emoji-picker-search")
                      .auto_focus(true)
                      .submit_behavior(SubmitBehavior::Submit)
                      .placeholder("搜索表情…")
                      .flex_1()
                      .min_w(px(0.0))
                      .text_size(px(13.0))
                      .text_color(move || palette(dark_mode.get()).text_primary)
                      .bg(move || palette(dark_mode.get()).surface_overlay)
                      .value(move || query.get())
                      .on_change(move |v: &str| query.set(v.to_owned()))
                      .on_submit(move |submitted: &str| {
                        if let Some(glyph) = first_matching_emoji(submitted) {
                          pick_submit(Some(glyph.to_string()));
                          close_submit();
                        }
                      })
                      .on_key_press(move |k: &str| {
                        if k == "Escape" {
                          close_input_esc();
                        }
                      }),
                  ),
              ),
            )
            // Subtle divider
            .child(
              view()
                .w_full()
                .h(px(1.0))
                .bg(move || palette(dark_mode.get()).border_subtle),
            )
            // Scrollable 8-column category grid or empty state
            .child(
              scroll().flex_1().min_h(px(0.0)).child(dynamic(move || {
                let q = query.get();
                let sections = filter_emoji_sections(&q);
                let cur = current_icon.get();
                let p = palette(dark_mode.get());

                if sections.is_empty() {
                  return view()
                    .w_full()
                    .h(px(290.0))
                    .px(px(26.0))
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(
                      view()
                        .w(px(46.0))
                        .h(px(46.0))
                        .rounded(px(8.0))
                        .bg(p.surface_sunken)
                        .items_center()
                        .justify_center()
                        .child(mica_icons::search(p.text_faint_hex)),
                    )
                    .child(
                      text("没有匹配的表情")
                        .text_size(px(14.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.text_primary),
                    )
                    .child(
                      text("换个关键词试试，或清空搜索查看全部图标")
                        .text_size(px(12.0))
                        .text_color(p.text_faint),
                    );
                }

                let mut body = view()
                  .w_full()
                  .pl(px(14.0))
                  .pr(px(14.0))
                  .pt(px(8.0))
                  .pb(px(12.0))
                  .gap(px(10.0));

                for (label, rows) in sections {
                  let mut section = view().w_full().gap(px(CELL_GAP));
                  section = section.child(
                    view().pb(px(3.0)).child(
                      text(label)
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.text_faint),
                    ),
                  );

                  for chunk in rows.chunks(GRID_COLUMNS) {
                    let mut grid_row = view().flex_row().items_center().gap(px(CELL_GAP));
                    for entry in chunk {
                      let glyph = entry.glyph;
                      let is_selected = cur.as_deref() == Some(glyph);
                      let pick_cell = pick_grid.clone();
                      let close_cell = close_grid.clone();
                      grid_row = grid_row.child(
                        button()
                          .test_id(&format!("emoji-cell-{glyph}"))
                          .accessibility_label(format!("选择图标 {glyph}"))
                          .w(px(CELL_SIZE))
                          .h(px(CELL_SIZE))
                          .items_center()
                          .justify_center()
                          .rounded(px(8.0))
                          .border(if is_selected { px(1.0) } else { px(0.0) })
                          .border_color(if is_selected {
                            p.text_primary
                          } else {
                            p.surface_overlay
                          })
                          .bg(if is_selected {
                            p.selection_bg
                          } else {
                            p.surface_overlay
                          })
                          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_sunken))
                          .on_press(move || {
                            pick_cell(Some(glyph.to_string()));
                            close_cell();
                          })
                          .child(text(glyph).text_size(px(20.0))),
                      );
                    }
                    section = section.child(grid_row);
                  }
                  body = body.child(section);
                }

                body
              })),
            ),
        )
    }
  })
}

// ---------------------------------------------------------------------------
// Curated bilingual table synced 1:1 from `emoji_picker.dart` (~260 emojis).
// ---------------------------------------------------------------------------

const SMILEYS: &[EmojiEntry] = &[
  e("😀", "grin smile 笑 微笑 开心"),
  e("😃", "smiley happy 高兴 笑"),
  e("😄", "laugh 大笑 开心"),
  e("😁", "beam grin 露齿 笑"),
  e("😆", "lol laughing 大笑 哈哈"),
  e("😅", "sweat smile 尴尬 汗"),
  e("🤣", "rofl 笑翻 爆笑"),
  e("😂", "joy tears 笑哭"),
  e("🙂", "slight smile 微笑"),
  e("🙃", "upside down 倒 无语"),
  e("😉", "wink 眨眼"),
  e("😊", "blush 害羞 微笑"),
  e("😇", "halo angel 天使"),
  e("🥰", "love hearts 喜欢 爱"),
  e("😍", "heart eyes 星星眼 爱"),
  e("🤩", "star struck 崇拜 惊叹"),
  e("😘", "kiss 亲"),
  e("😋", "yum tasty 好吃 美味"),
  e("😛", "tongue 舌头 调皮"),
  e("🤪", "zany 疯 搞怪"),
  e("🤨", "raised eyebrow 疑惑 怀疑"),
  e("🧐", "monocle 审视 研究"),
  e("🤓", "nerd 书呆 学霸"),
  e("😎", "cool sunglasses 酷 墨镜"),
  e("🤔", "thinking 思考 想"),
  e("🤫", "shush quiet 安静 保密"),
  e("🤐", "zipper mouth 闭嘴 保密"),
  e("😐", "neutral 无表情 平静"),
  e("😑", "expressionless 无语"),
  e("😶", "no mouth 沉默"),
  e("😏", "smirk 得意 坏笑"),
  e("😒", "unamused 不爽 无语"),
  e("🙄", "eye roll 白眼"),
  e("😬", "grimace 尴尬"),
  e("😔", "pensive 失落 难过"),
  e("😴", "sleeping 睡 睡觉"),
  e("😌", "relieved 放松 安心"),
  e("😢", "cry 哭 难过"),
  e("😭", "sob 大哭 崩溃"),
  e("😤", "triumph huff 生气 不服"),
  e("😠", "angry 生气"),
  e("😡", "rage 愤怒 火大"),
  e("🤯", "mind blown 震惊 爆炸"),
  e("😱", "scream 惊恐 害怕"),
  e("🥳", "party face 庆祝 派对"),
  e("🤗", "hug 抱 拥抱"),
  e("🥺", "pleading 委屈 求"),
  e("👻", "ghost 鬼 幽灵"),
  e("💀", "skull 骷髅 死"),
  e("👽", "alien 外星人"),
  e("🤖", "robot bot 机器人 ai"),
];

const PEOPLE: &[EmojiEntry] = &[
  e("👍", "thumbs up 赞 好 支持"),
  e("👎", "thumbs down 差 反对"),
  e("👌", "ok hand 好 可以"),
  e("✌️", "victory peace 胜利 剪刀手"),
  e("🤞", "fingers crossed 祈祷 好运"),
  e("👏", "clap 鼓掌 赞"),
  e("🙌", "raised hands 欢呼 举手"),
  e("🙏", "pray thanks 拜托 感谢 祈祷"),
  e("🤝", "handshake 握手 合作"),
  e("💪", "muscle strong 加油 力量"),
  e("✍️", "writing hand 写 手写"),
  e("👀", "eyes look 看 关注"),
  e("🧠", "brain 大脑 思考 智力"),
  e("👤", "person user 用户 人"),
  e("👥", "people users 用户 团队"),
  e("🙋", "raising hand 提问 举手"),
  e("🤷", "shrug 摊手 不知道"),
  e("🧑‍💻", "technologist coder 程序员 开发"),
  e("👨‍🏫", "teacher 老师 教学"),
  e("🕵️", "detective 侦探 调查"),
  e("🦸", "superhero 超人 英雄"),
  e("🧙", "wizard mage 巫师 魔法"),
  e("🏃", "running 跑 跑步"),
  e("🧘", "meditation yoga 冥想 瑜伽"),
];

const NATURE: &[EmojiEntry] = &[
  e("🌱", "seedling sprout 幼苗 发芽 成长"),
  e("🌿", "herb leaf 叶 植物"),
  e("🍀", "clover luck 四叶草 幸运"),
  e("🌳", "tree 树"),
  e("🌲", "evergreen pine 松树 森林"),
  e("🌵", "cactus 仙人掌"),
  e("🌻", "sunflower 向日葵 花"),
  e("🌸", "blossom cherry 樱花 花"),
  e("🌹", "rose 玫瑰 花"),
  e("🍁", "maple leaf 枫叶 秋"),
  e("🌊", "wave ocean 海浪 水"),
  e("💧", "droplet water 水滴 水"),
  e("🔥", "fire flame 火 火焰 热门"),
  e("⭐", "star 星 收藏"),
  e("🌟", "glowing star 闪耀 星"),
  e("✨", "sparkles 闪 亮 新"),
  e("⚡", "zap lightning 闪电 快"),
  e("☀️", "sun sunny 太阳 晴"),
  e("☁️", "cloud 云"),
  e("🌧️", "rain 雨"),
  e("❄️", "snowflake 雪 冬"),
  e("🌈", "rainbow 彩虹"),
  e("🌙", "moon crescent 月亮 夜"),
  e("🌍", "earth globe 地球 世界"),
  e("🪐", "planet saturn 行星 土星"),
  e("🐶", "dog puppy 狗 小狗"),
  e("🐱", "cat 猫"),
  e("🦊", "fox 狐狸"),
  e("🐻", "bear 熊"),
  e("🐼", "panda 熊猫"),
  e("🦁", "lion 狮子"),
  e("🦉", "owl 猫头鹰 夜"),
  e("🐝", "bee 蜜蜂"),
  e("🐛", "bug caterpillar 虫 缺陷"),
  e("🦋", "butterfly 蝴蝶"),
  e("🐙", "octopus 章鱼"),
  e("🐳", "whale 鲸鱼"),
  e("🦖", "dinosaur 恐龙"),
];

const FOOD: &[EmojiEntry] = &[
  e("☕", "coffee 咖啡"),
  e("🍵", "tea 茶"),
  e("🧊", "ice 冰"),
  e("🍺", "beer 啤酒"),
  e("🍷", "wine 红酒"),
  e("🥂", "cheers toast 干杯 庆祝"),
  e("🍎", "apple 苹果"),
  e("🍊", "orange tangerine 橙子 橘子"),
  e("🍋", "lemon 柠檬"),
  e("🍌", "banana 香蕉"),
  e("🍉", "watermelon 西瓜"),
  e("🍇", "grapes 葡萄"),
  e("🍓", "strawberry 草莓"),
  e("🥑", "avocado 牛油果"),
  e("🍞", "bread 面包"),
  e("🥐", "croissant 可颂"),
  e("🧀", "cheese 奶酪"),
  e("🍕", "pizza 披萨"),
  e("🍔", "burger 汉堡"),
  e("🍜", "noodles ramen 面 拉面"),
  e("🍚", "rice 米饭"),
  e("🍣", "sushi 寿司"),
  e("🍰", "cake slice 蛋糕"),
  e("🎂", "birthday cake 生日 蛋糕"),
  e("🍪", "cookie 饼干"),
  e("🍫", "chocolate 巧克力"),
  e("🍿", "popcorn 爆米花"),
  e("🥗", "salad 沙拉"),
  e("🍄", "mushroom 蘑菇"),
];

const OBJECTS: &[EmojiEntry] = &[
  e("📗", "book green 书 笔记 绿"),
  e("📘", "book blue 书 蓝"),
  e("📕", "book red 书 红"),
  e("📙", "book orange 书 橙"),
  e("📚", "books library 书 书架 资料"),
  e("📖", "open book read 阅读 读书"),
  e("📔", "notebook 笔记本"),
  e("📒", "ledger 账本 笔记"),
  e("📝", "memo note 笔记 记录 便签"),
  e("📄", "page document 文档 页"),
  e("📑", "bookmark tabs 书签 标签"),
  e("📋", "clipboard 剪贴板 清单"),
  e("📌", "pushpin 图钉 置顶"),
  e("📍", "location pin 位置 定位"),
  e("📎", "paperclip attachment 回形针 附件"),
  e("✂️", "scissors cut 剪刀 剪切"),
  e("✏️", "pencil edit 铅笔 编辑"),
  e("🎨", "art palette 设计 艺术 配色"),
  e("📐", "set square 尺 设计"),
  e("🔍", "search magnifier 搜索 查找"),
  e("🔒", "lock 锁 私密 安全"),
  e("🔓", "unlock 解锁 公开"),
  e("🔑", "key 钥匙 密钥"),
  e("🛠️", "tools 工具 维护"),
  e("🔧", "wrench 扳手 配置"),
  e("⚙️", "gear settings 设置 配置 齿轮"),
  e("🧰", "toolbox 工具箱"),
  e("🧪", "test tube 实验 测试"),
  e("🔬", "microscope 显微镜 研究"),
  e("🔭", "telescope 望远镜 探索"),
  e("💡", "idea bulb 灯泡 想法 点子"),
  e("🔔", "bell 铃 通知"),
  e("📣", "megaphone 喇叭 公告"),
  e("📅", "calendar 日历 日程"),
  e("⏰", "alarm clock 闹钟 提醒"),
  e("⏳", "hourglass 沙漏 等待"),
  e("💻", "laptop computer 电脑 笔记本 开发"),
  e("🖥️", "desktop monitor 显示器 电脑"),
  e("⌨️", "keyboard 键盘"),
  e("📱", "phone mobile 手机 移动"),
  e("💾", "floppy save 保存 磁盘"),
  e("🗄️", "file cabinet 文件柜 归档"),
  e("🗂️", "card index 分类 索引"),
  e("📁", "folder 文件夹 目录"),
  e("📂", "open folder 文件夹 打开"),
  e("🗑️", "trash bin 垃圾桶 删除"),
  e("📦", "package box 包 打包 归档"),
  e("🎁", "gift 礼物"),
  e("🏷️", "label tag 标签"),
  e("🔖", "bookmark 书签"),
  e("📊", "bar chart 图表 数据 统计"),
  e("📈", "chart up 增长 上升 数据"),
  e("📉", "chart down 下降 数据"),
  e("🧾", "receipt 收据 账单"),
  e("💰", "money bag 钱 预算"),
  e("🏠", "house home 家 首页"),
  e("🏢", "office building 公司 办公"),
  e("🚀", "rocket launch 火箭 发布 上线"),
  e("✈️", "airplane 飞机 出行"),
  e("🧭", "compass 指南针 方向"),
  e("🗺️", "map 地图 规划"),
  e("🎯", "target dart 目标 靶"),
  e("🏆", "trophy 奖杯 成就"),
  e("🥇", "gold medal 第一 金牌"),
  e("🎓", "graduation cap 毕业 学习"),
  e("🎉", "party popper 庆祝 完成"),
  e("🛡️", "shield 盾 安全 防护"),
  e("⚖️", "scales balance 天平 权衡"),
  e("🔗", "link chain 链接 关联"),
  e("🧩", "puzzle piece 拼图 模块"),
  e("🎮", "game controller 游戏"),
  e("🎧", "headphones 耳机 音乐"),
  e("🎬", "clapper film 电影 视频"),
  e("📷", "camera photo 相机 照片"),
  e("🎵", "music note 音乐"),
  e("🖼️", "framed picture 图片 画"),
  e("✉️", "envelope mail 邮件 信"),
  e("🔋", "battery 电池 电量"),
  e("🔌", "plug 插头 电源"),
  e("🚪", "door 门 入口"),
];

const SYMBOLS: &[EmojiEntry] = &[
  e("✅", "check done 完成 对 勾"),
  e("☑️", "checkbox ticked 勾选 待办"),
  e("❌", "cross wrong 错 取消 失败"),
  e("⭕", "circle right 圈 正确"),
  e("❗", "exclamation 重要 注意"),
  e("❓", "question 问题 疑问"),
  e("⚠️", "warning caution 警告 注意"),
  e("🚫", "prohibited 禁止 禁用"),
  e("🔴", "red circle 红 停 严重"),
  e("🟠", "orange circle 橙 中等"),
  e("🟡", "yellow circle 黄 提醒"),
  e("🟢", "green circle 绿 正常 通过"),
  e("🔵", "blue circle 蓝"),
  e("🟣", "purple circle 紫"),
  e("⚫", "black circle 黑"),
  e("⚪", "white circle 白"),
  e("🔺", "red triangle up 三角 上升"),
  e("🔻", "red triangle down 三角 下降"),
  e("🔶", "orange diamond 菱形"),
  e("🔷", "blue diamond 菱形"),
  e("♻️", "recycle 回收 复用"),
  e("♾️", "infinity 无限"),
  e("▶️", "play start 播放 开始"),
  e("⏸️", "pause 暂停"),
  e("🔄", "refresh sync 刷新 同步"),
  e("➕", "plus add 加 新增"),
  e("➖", "minus 减 移除"),
  e("💬", "speech bubble 评论 对话 消息"),
  e("💭", "thought bubble 想法 思考"),
  e("❤️", "red heart 心 爱 喜欢"),
  e("🧡", "orange heart 心 橙"),
  e("💛", "yellow heart 心 黄"),
  e("💚", "green heart 心 绿"),
  e("💙", "blue heart 心 蓝"),
  e("💜", "purple heart 心 紫"),
  e("💯", "hundred points 满分 一百"),
  e("🆕", "new 新 新建"),
  e("🔝", "top 置顶 最上"),
];

const FLAGS: &[EmojiEntry] = &[
  e("🚩", "red flag 旗 标记 重要"),
  e("🏁", "chequered flag 终点 完成 比赛"),
  e("🏳️", "white flag 白旗 投降"),
  e("🏴", "black flag 黑旗"),
  e("🎌", "crossed flags 旗 交叉"),
  e("🏴‍☠️", "pirate flag 海盗旗"),
  e("⛳", "golf flag 高尔夫 目标"),
  e("🎏", "carp streamer 鲤鱼旗"),
];

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn curated_catalog_has_7_categories_and_over_120_emojis() {
    assert_eq!(EMOJI_CATEGORIES.len(), 7);
    let total: usize = EMOJI_CATEGORIES.iter().map(|(_, list)| list.len()).sum();
    assert!(total >= 120, "expected >= 120 curated emojis, got {total}");
  }

  #[test]
  fn bilingual_chinese_and_english_search_and_enter_first_match() {
    let zh = filter_emoji_sections("火箭");
    assert_eq!(zh.len(), 1);
    assert_eq!(zh[0].0, "物件");
    assert_eq!(zh[0].1[0].glyph, "🚀");

    let en = filter_emoji_sections("ROCKET");
    assert_eq!(first_matching_emoji("ROCKET"), Some("🚀"));
    assert_eq!(en[0].1[0].glyph, "🚀");

    // Empty search must not pick anything on Enter
    assert_eq!(first_matching_emoji(""), None);
    assert_eq!(first_matching_emoji("   "), None);

    // Unmatched search returns empty sections
    assert!(filter_emoji_sections("zzz_no_such_emoji_999").is_empty());
  }
}
