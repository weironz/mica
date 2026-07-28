import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'theme_tokens.dart';

/// Every user-visible string in the picker. Passed in rather than looked up so
/// this file stays free of both hardcoded copy and a dependency on the app's
/// l10n layer — the host owns translation, the picker owns behaviour.
@immutable
class EmojiPickerStrings {
  const EmojiPickerStrings({
    required this.title,
    required this.searchHint,
    required this.removeIcon,
    required this.noResultsTitle,
    required this.noResultsBody,
    required this.categorySmileys,
    required this.categoryPeople,
    required this.categoryNature,
    required this.categoryFood,
    required this.categoryObjects,
    required this.categorySymbols,
    required this.categoryFlags,
  });

  /// Popup header, e.g. "选择图标".
  final String title;
  final String searchHint;

  /// The clear affordance. Choosing it resolves the future with `''`.
  final String removeIcon;

  /// Empty state: what happened + what to do next. Never "出错了" — a search that
  /// matched nothing is not a failure (docs/design/…/19 空状态与故障态).
  final String noResultsTitle;
  final String noResultsBody;

  final String categorySmileys;
  final String categoryPeople;
  final String categoryNature;
  final String categoryFood;
  final String categoryObjects;
  final String categorySymbols;
  final String categoryFlags;
}

/// Returns the chosen emoji, `''` to CLEAR the current icon, or null if the user
/// dismissed the picker without choosing. The three-way result mirrors the
/// server's icon semantics (absent = leave alone, '' = clear, emoji = set), so
/// callers must not collapse `''` into null on the way to the API.
Future<String?> showEmojiPicker(
  BuildContext context, {
  required EmojiPickerStrings strings,
  String? current,
}) {
  return showDialog<String>(
    context: context,
    // Both ways out of the popup without a decision (barrier tap, Esc) pop with
    // no value, which showDialog surfaces as null = "leave the icon alone".
    builder: (_) => _EmojiPicker(strings: strings, current: current),
  );
}

// Container border (#E5E9F0) and inner divider (#EEF1F5) from the visual spec.
// Not in EditorTheme — that one only carries canvas ink/wash colours.
Color _border(BuildContext context) => MicaTheme.of(context).border.normal;
Color _divider(BuildContext context) => MicaTheme.of(context).border.subtle;

/// Cell side and gap: 8 columns must fit the 360-wide popup minus its padding
/// (8×38 + 7×3 = 325 ≤ 332), and 38 keeps the tap target well over 32.
const double _cell = 38;
const double _gap = 3;

class _EmojiPicker extends StatefulWidget {
  const _EmojiPicker({required this.strings, this.current});

  final EmojiPickerStrings strings;
  final String? current;

  @override
  State<_EmojiPicker> createState() => _EmojiPickerState();
}

class _EmojiPickerState extends State<_EmojiPicker> {
  final _query = TextEditingController();

  /// Lowercased trimmed query, kept in state so build() doesn't re-derive it per
  /// section (there are a few hundred rows to walk).
  String _needle = '';

  @override
  void dispose() {
    _query.dispose();
    super.dispose();
  }

  bool _matches(_E row) =>
      _needle.isEmpty || row.$1 == _needle || row.$2.contains(_needle);

  /// Sections in table order, each reduced to the rows that match, empty ones
  /// dropped — so a filtered view shows only the headers that still have hits.
  List<(String, List<_E>)> _sections() {
    final s = widget.strings;
    final table = <(String, List<_E>)>[
      (s.categorySmileys, _kSmileys),
      (s.categoryPeople, _kPeople),
      (s.categoryNature, _kNature),
      (s.categoryFood, _kFood),
      (s.categoryObjects, _kObjects),
      (s.categorySymbols, _kSymbols),
      (s.categoryFlags, _kFlags),
    ];
    final out = <(String, List<_E>)>[];
    for (final (label, rows) in table) {
      final hits = rows.where(_matches).toList(growable: false);
      if (hits.isNotEmpty) out.add((label, hits));
    }
    return out;
  }

  void _pick(String emoji) => Navigator.of(context).pop(emoji);

  /// Enter takes the first match — the reason someone types is to reach one
  /// emoji fast. Deliberately does nothing on an empty box: there Enter would
  /// set whatever glyph happens to sit first in the table, which reads as a bug.
  void _pickFirst() {
    if (_needle.isEmpty) return;
    final sections = _sections();
    if (sections.isEmpty) return;
    _pick(sections.first.$2.first.$1);
  }

  @override
  Widget build(BuildContext context) {
    final s = widget.strings;
    final sections = _sections();
    return Dialog(
      // The card below draws its own surface so the shadow/border/radius match
      // the popup spec exactly (radius 16, #E5E9F0, soft high offset shadow).
      backgroundColor: Colors.transparent,
      elevation: 0,
      insetPadding: const EdgeInsets.all(24),
      child: CallbackShortcuts(
        bindings: {
          // Handled here rather than left to the route so Esc means the same
          // thing whether or not the search field has focus.
          const SingleActivator(LogicalKeyboardKey.escape): () =>
              Navigator.of(context).pop(),
        },
        child: Container(
          width: 360,
          height: 420,
          decoration: BoxDecoration(
            color: MicaTheme.of(context).surface.overlay,
            border: Border.all(color: _border(context)),
            borderRadius: const BorderRadius.all(Radius.circular(16)),
            boxShadow: const [
              BoxShadow(
                color: Color(0x730F172A),
                blurRadius: 70,
                spreadRadius: -45,
                offset: Offset(0, 30),
              ),
            ],
          ),
          child: Column(
            children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(14, 12, 10, 10),
                child: Row(
                  children: [
                    Expanded(
                      child: Text(
                        s.title,
                        style: TextStyle(
                          fontSize: 14,
                          fontWeight: FontWeight.w600,
                          color: MicaTheme.of(context).text.primary,
                        ),
                      ),
                    ),
                    // Always offered, not only when an icon is set: clearing
                    // nothing is harmless, and one less state beats a control
                    // that appears and disappears under the cursor.
                    _RemoveAction(
                      label: s.removeIcon,
                      onTap: () => Navigator.of(context).pop(''),
                    ),
                  ],
                ),
              ),
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 14),
                child: TextField(
                  controller: _query,
                  autofocus: true,
                  style: TextStyle(
                    fontSize: 13,
                    color: MicaTheme.of(context).text.primary,
                  ),
                  cursorColor: MicaTheme.of(context).accent.primary,
                  onChanged: (value) =>
                      setState(() => _needle = value.trim().toLowerCase()),
                  onSubmitted: (_) => _pickFirst(),
                  decoration: InputDecoration(
                    isDense: true,
                    hintText: s.searchHint,
                    hintStyle: TextStyle(
                      fontSize: 13,
                      color: MicaTheme.of(context).text.faint,
                    ),
                    prefixIcon: Icon(
                      Icons.search,
                      size: 15,
                      color: MicaTheme.of(context).text.faint,
                    ),
                    prefixIconConstraints: const BoxConstraints(
                      minWidth: 32,
                      minHeight: 32,
                    ),
                    contentPadding: const EdgeInsets.only(right: 10, bottom: 9),
                    enabledBorder: OutlineInputBorder(
                      borderRadius: BorderRadius.all(Radius.circular(8)),
                      borderSide: BorderSide(color: _border(context)),
                    ),
                    focusedBorder: OutlineInputBorder(
                      borderRadius: const BorderRadius.all(Radius.circular(8)),
                      borderSide: BorderSide(
                        color: MicaTheme.of(context).accent.primary,
                      ),
                    ),
                  ),
                ),
              ),
              Padding(
                padding: EdgeInsets.only(top: 10),
                child: Divider(
                  height: 1,
                  thickness: 1,
                  color: _divider(context),
                ),
              ),
              Expanded(
                child: sections.isEmpty
                    ? _NoResults(title: s.noResultsTitle, body: s.noResultsBody)
                    : ListView(
                        padding: const EdgeInsets.fromLTRB(14, 8, 14, 12),
                        children: [
                          for (final (label, rows) in sections)
                            _Section(
                              label: label,
                              rows: rows,
                              current: widget.current,
                              onPick: _pick,
                            ),
                        ],
                      ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _Section extends StatelessWidget {
  const _Section({
    required this.label,
    required this.rows,
    required this.current,
    required this.onPick,
  });

  final String label;
  final List<_E> rows;
  final String? current;
  final void Function(String emoji) onPick;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: const EdgeInsets.only(bottom: 6, top: 2),
          child: Text(
            label,
            style: TextStyle(
              fontSize: 12,
              fontWeight: FontWeight.w600,
              color: MicaTheme.of(context).text.faint,
            ),
          ),
        ),
        Wrap(
          spacing: _gap,
          runSpacing: _gap,
          children: [
            for (final row in rows)
              _EmojiCell(
                emoji: row.$1,
                selected: row.$1 == current,
                onTap: () => onPick(row.$1),
              ),
          ],
        ),
        const SizedBox(height: 10),
      ],
    );
  }
}

class _EmojiCell extends StatefulWidget {
  const _EmojiCell({
    required this.emoji,
    required this.selected,
    required this.onTap,
  });

  final String emoji;
  final bool selected;
  final VoidCallback onTap;

  @override
  State<_EmojiCell> createState() => _EmojiCellState();
}

class _EmojiCellState extends State<_EmojiCell> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        // The cell is mostly empty space around one glyph; deferToChild would
        // only accept taps that land on the glyph itself.
        behavior: HitTestBehavior.opaque,
        onTap: widget.onTap,
        child: Container(
          key: ValueKey('emoji-cell-${widget.emoji}'),
          width: _cell,
          height: _cell,
          alignment: Alignment.center,
          decoration: BoxDecoration(
            color: widget.selected
                ? MicaTheme.of(context).editor.selection
                : (_hovered ? MicaTheme.of(context).surface.sunken : null),
            borderRadius: const BorderRadius.all(Radius.circular(8)),
            border: widget.selected
                ? Border.all(color: MicaTheme.of(context).accent.primary)
                : null,
          ),
          child: Text(widget.emoji, style: const TextStyle(fontSize: 20)),
        ),
      ),
    );
  }
}

class _RemoveAction extends StatelessWidget {
  const _RemoveAction({required this.label, required this.onTap});

  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Container(
          key: const ValueKey('emoji-picker-remove'),
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
          decoration: BoxDecoration(
            border: Border.all(color: _border(context)),
            borderRadius: const BorderRadius.all(Radius.circular(8)),
          ),
          child: Text(
            label,
            style: TextStyle(
              fontSize: 12,
              color: MicaTheme.of(context).text.muted,
            ),
          ),
        ),
      ),
    );
  }
}

/// A search that matched nothing: say what happened and give a next step. Not a
/// failure state, so no red and no "出错了".
class _NoResults extends StatelessWidget {
  const _NoResults({required this.title, required this.body});

  final String title;
  final String body;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 26),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: 46,
              height: 46,
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: MicaTheme.of(context).surface.sunken,
                borderRadius: BorderRadius.all(Radius.circular(13)),
              ),
              child: Icon(
                Icons.search_off,
                size: 21,
                color: MicaTheme.of(context).text.faint,
              ),
            ),
            const SizedBox(height: 11),
            Text(
              title,
              textAlign: TextAlign.center,
              style: TextStyle(
                fontSize: 14,
                fontWeight: FontWeight.w600,
                color: MicaTheme.of(context).text.primary,
              ),
            ),
            const SizedBox(height: 6),
            Text(
              body,
              textAlign: TextAlign.center,
              style: TextStyle(
                fontSize: 12.5,
                height: 1.75,
                color: MicaTheme.of(context).text.faint,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// The table. (glyph, keywords) — keywords are one lowercase haystack matched by
// substring, and carry BOTH English and Chinese because the UI is primarily
// Chinese. Deliberately a curated few hundred glyphs, not the Unicode set: a
// page icon is picked from what people actually reach for, and shipping the full
// list would mean an emoji data package (a dependency this project won't take).
// Every glyph appears exactly once across the sections — the cells key off it.
// ---------------------------------------------------------------------------

typedef _E = (String, String);

const List<_E> _kSmileys = [
  ('😀', 'grin smile 笑 微笑 开心'),
  ('😃', 'smiley happy 高兴 笑'),
  ('😄', 'laugh 大笑 开心'),
  ('😁', 'beam grin 露齿 笑'),
  ('😆', 'lol laughing 大笑 哈哈'),
  ('😅', 'sweat smile 尴尬 汗'),
  ('🤣', 'rofl 笑翻 爆笑'),
  ('😂', 'joy tears 笑哭'),
  ('🙂', 'slight smile 微笑'),
  ('🙃', 'upside down 倒 无语'),
  ('😉', 'wink 眨眼'),
  ('😊', 'blush 害羞 微笑'),
  ('😇', 'halo angel 天使'),
  ('🥰', 'love hearts 喜欢 爱'),
  ('😍', 'heart eyes 星星眼 爱'),
  ('🤩', 'star struck 崇拜 惊叹'),
  ('😘', 'kiss 亲'),
  ('😋', 'yum tasty 好吃 美味'),
  ('😛', 'tongue 舌头 调皮'),
  ('🤪', 'zany 疯 搞怪'),
  ('🤨', 'raised eyebrow 疑惑 怀疑'),
  ('🧐', 'monocle 审视 研究'),
  ('🤓', 'nerd 书呆 学霸'),
  ('😎', 'cool sunglasses 酷 墨镜'),
  ('🤔', 'thinking 思考 想'),
  ('🤫', 'shush quiet 安静 保密'),
  ('🤐', 'zipper mouth 闭嘴 保密'),
  ('😐', 'neutral 无表情 平静'),
  ('😑', 'expressionless 无语'),
  ('😶', 'no mouth 沉默'),
  ('😏', 'smirk 得意 坏笑'),
  ('😒', 'unamused 不爽 无语'),
  ('🙄', 'eye roll 白眼'),
  ('😬', 'grimace 尴尬'),
  ('😔', 'pensive 失落 难过'),
  ('😪', 'sleepy 困 打盹'),
  ('😴', 'sleeping 睡 睡觉'),
  ('🥱', 'yawn 打哈欠 累'),
  ('😌', 'relieved 放松 安心'),
  ('😢', 'cry 哭 难过'),
  ('😭', 'sob 大哭 崩溃'),
  ('😤', 'triumph huff 生气 不服'),
  ('😠', 'angry 生气'),
  ('😡', 'rage 愤怒 火大'),
  ('🤯', 'mind blown 震惊 爆炸'),
  ('😱', 'scream 惊恐 害怕'),
  ('😳', 'flushed 脸红 害羞'),
  ('🥳', 'party face 庆祝 派对'),
  ('🤗', 'hug 抱 拥抱'),
  ('🥺', 'pleading 委屈 求'),
  ('😷', 'mask 口罩 生病'),
  ('🤒', 'sick fever 发烧 生病'),
  ('🤕', 'injured bandage 受伤'),
  ('🥶', 'cold freezing 冷 冻'),
  ('🥵', 'hot overheated 热'),
  ('👻', 'ghost 鬼 幽灵'),
  ('💀', 'skull 骷髅 死'),
  ('👽', 'alien 外星人'),
  ('🤖', 'robot bot 机器人 ai'),
];

const List<_E> _kPeople = [
  ('👍', 'thumbs up 赞 好 支持'),
  ('👎', 'thumbs down 差 反对'),
  ('👌', 'ok hand 好 可以'),
  ('✌️', 'victory peace 胜利 剪刀手'),
  ('🤞', 'fingers crossed 祈祷 好运'),
  ('🤙', 'call me 打电话'),
  ('👏', 'clap 鼓掌 赞'),
  ('🙌', 'raised hands 欢呼 举手'),
  ('🙏', 'pray thanks 拜托 感谢 祈祷'),
  ('🤝', 'handshake 握手 合作'),
  ('💪', 'muscle strong 加油 力量'),
  ('✍️', 'writing hand 写 手写'),
  ('👀', 'eyes look 看 关注'),
  ('🧠', 'brain 大脑 思考 智力'),
  ('👤', 'person user 用户 人'),
  ('👥', 'people users 用户 团队'),
  ('🙋', 'raising hand 提问 举手'),
  ('🤷', 'shrug 摊手 不知道'),
  ('🧑‍💻', 'technologist coder 程序员 开发'),
  ('👨‍🏫', 'teacher 老师 教学'),
  ('🕵️', 'detective 侦探 调查'),
  ('👮', 'police 警察'),
  ('🦸', 'superhero 超人 英雄'),
  ('🧙', 'wizard mage 巫师 魔法'),
  ('👶', 'baby 婴儿 宝宝'),
  ('🚶', 'walking 走 散步'),
  ('🏃', 'running 跑 跑步'),
  ('🧘', 'meditation yoga 冥想 瑜伽'),
  ('💃', 'dancing 跳舞'),
];

const List<_E> _kNature = [
  ('🌱', 'seedling sprout 幼苗 发芽 成长'),
  ('🌿', 'herb leaf 叶 植物'),
  ('🍀', 'clover luck 四叶草 幸运'),
  ('🌳', 'tree 树'),
  ('🌲', 'evergreen pine 松树 森林'),
  ('🌵', 'cactus 仙人掌'),
  ('🌻', 'sunflower 向日葵 花'),
  ('🌸', 'blossom cherry 樱花 花'),
  ('🌹', 'rose 玫瑰 花'),
  ('💐', 'bouquet 花束'),
  ('🍁', 'maple leaf 枫叶 秋'),
  ('🍂', 'fallen leaves 落叶 秋'),
  ('🌊', 'wave ocean 海浪 水'),
  ('💧', 'droplet water 水滴 水'),
  ('🔥', 'fire flame 火 火焰 热门'),
  ('⭐', 'star 星 收藏'),
  ('🌟', 'glowing star 闪耀 星'),
  ('✨', 'sparkles 闪 亮 新'),
  ('⚡', 'zap lightning 闪电 快'),
  ('☀️', 'sun sunny 太阳 晴'),
  ('☁️', 'cloud 云'),
  ('🌧️', 'rain 雨'),
  ('⛈️', 'storm thunder 雷雨 暴风'),
  ('❄️', 'snowflake 雪 冬'),
  ('🌈', 'rainbow 彩虹'),
  ('🌙', 'moon crescent 月亮 夜'),
  ('🌍', 'earth globe 地球 世界'),
  ('🪐', 'planet saturn 行星 土星'),
  ('🐶', 'dog puppy 狗 小狗'),
  ('🐱', 'cat 猫'),
  ('😺', 'cat smile 猫 笑'),
  ('🐭', 'mouse 鼠'),
  ('🐰', 'rabbit bunny 兔'),
  ('🦊', 'fox 狐狸'),
  ('🐻', 'bear 熊'),
  ('🐼', 'panda 熊猫'),
  ('🐨', 'koala 考拉'),
  ('🐯', 'tiger 虎 老虎'),
  ('🦁', 'lion 狮子'),
  ('🐮', 'cow 牛'),
  ('🐷', 'pig 猪'),
  ('🐸', 'frog 青蛙'),
  ('🐵', 'monkey 猴'),
  ('🐔', 'chicken 鸡'),
  ('🐦', 'bird 鸟'),
  ('🦆', 'duck 鸭'),
  ('🦉', 'owl 猫头鹰 夜'),
  ('🐝', 'bee 蜜蜂'),
  ('🐛', 'bug caterpillar 虫 缺陷'),
  ('🐞', 'ladybug 瓢虫'),
  ('🦋', 'butterfly 蝴蝶'),
  ('🐢', 'turtle 乌龟 慢'),
  ('🐍', 'snake python 蛇'),
  ('🐙', 'octopus 章鱼'),
  ('🐳', 'whale 鲸鱼'),
  ('🐬', 'dolphin 海豚'),
  ('🐟', 'fish 鱼'),
  ('🦖', 'dinosaur 恐龙'),
];

const List<_E> _kFood = [
  ('☕', 'coffee 咖啡'),
  ('🍵', 'tea 茶'),
  ('🧊', 'ice 冰'),
  ('🍺', 'beer 啤酒'),
  ('🍷', 'wine 红酒'),
  ('🥂', 'cheers toast 干杯 庆祝'),
  ('🍎', 'apple 苹果'),
  ('🍊', 'orange tangerine 橙子 橘子'),
  ('🍋', 'lemon 柠檬'),
  ('🍌', 'banana 香蕉'),
  ('🍉', 'watermelon 西瓜'),
  ('🍇', 'grapes 葡萄'),
  ('🍓', 'strawberry 草莓'),
  ('🍒', 'cherries 樱桃'),
  ('🥑', 'avocado 牛油果'),
  ('🍞', 'bread 面包'),
  ('🥐', 'croissant 可颂'),
  ('🧀', 'cheese 奶酪'),
  ('🍕', 'pizza 披萨'),
  ('🍔', 'burger 汉堡'),
  ('🍟', 'fries 薯条'),
  ('🌮', 'taco 墨西哥卷'),
  ('🍜', 'noodles ramen 面 拉面'),
  ('🍚', 'rice 米饭'),
  ('🍣', 'sushi 寿司'),
  ('🍰', 'cake slice 蛋糕'),
  ('🎂', 'birthday cake 生日 蛋糕'),
  ('🍪', 'cookie 饼干'),
  ('🍫', 'chocolate 巧克力'),
  ('🍬', 'candy 糖'),
  ('🍿', 'popcorn 爆米花'),
  ('🥗', 'salad 沙拉'),
  ('🥕', 'carrot 胡萝卜'),
  ('🌽', 'corn 玉米'),
  ('🍄', 'mushroom 蘑菇'),
  ('🥚', 'egg 蛋'),
];

const List<_E> _kObjects = [
  ('📗', 'book green 书 笔记 绿'),
  ('📘', 'book blue 书 蓝'),
  ('📕', 'book red 书 红'),
  ('📙', 'book orange 书 橙'),
  ('📚', 'books library 书 书架 资料'),
  ('📖', 'open book read 阅读 读书'),
  ('📔', 'notebook 笔记本'),
  ('📒', 'ledger 账本 笔记'),
  ('📝', 'memo note 笔记 记录 便签'),
  ('📄', 'page document 文档 页'),
  ('📑', 'bookmark tabs 书签 标签'),
  ('📋', 'clipboard 剪贴板 清单'),
  ('📌', 'pushpin 图钉 置顶'),
  ('📍', 'location pin 位置 定位'),
  ('📎', 'paperclip attachment 回形针 附件'),
  ('✂️', 'scissors cut 剪刀 剪切'),
  ('🖊️', 'pen 笔'),
  ('✏️', 'pencil edit 铅笔 编辑'),
  ('🖌️', 'paintbrush 画笔 设计'),
  ('🎨', 'art palette 设计 艺术 配色'),
  ('📐', 'set square 尺 设计'),
  ('📏', 'ruler 直尺 尺寸'),
  ('🔍', 'search magnifier 搜索 查找'),
  ('🔒', 'lock 锁 私密 安全'),
  ('🔓', 'unlock 解锁 公开'),
  ('🔑', 'key 钥匙 密钥'),
  ('🔨', 'hammer 锤 工具'),
  ('🛠️', 'tools 工具 维护'),
  ('🔧', 'wrench 扳手 配置'),
  ('⚙️', 'gear settings 设置 配置 齿轮'),
  ('🧰', 'toolbox 工具箱'),
  ('🧪', 'test tube 实验 测试'),
  ('🔬', 'microscope 显微镜 研究'),
  ('🔭', 'telescope 望远镜 探索'),
  ('💡', 'idea bulb 灯泡 想法 点子'),
  ('🔔', 'bell 铃 通知'),
  ('📣', 'megaphone 喇叭 公告'),
  ('📅', 'calendar 日历 日程'),
  ('⏰', 'alarm clock 闹钟 提醒'),
  ('⏳', 'hourglass 沙漏 等待'),
  ('🕐', 'clock time 时间 时钟'),
  ('💻', 'laptop computer 电脑 笔记本 开发'),
  ('🖥️', 'desktop monitor 显示器 电脑'),
  ('⌨️', 'keyboard 键盘'),
  ('🖱️', 'computer mouse 鼠标'),
  ('📱', 'phone mobile 手机 移动'),
  ('🖨️', 'printer 打印机'),
  ('💾', 'floppy save 保存 磁盘'),
  ('💿', 'disc cd 光盘'),
  ('🗄️', 'file cabinet 文件柜 归档'),
  ('🗂️', 'card index 分类 索引'),
  ('📁', 'folder 文件夹 目录'),
  ('📂', 'open folder 文件夹 打开'),
  ('🗑️', 'trash bin 垃圾桶 删除'),
  ('📦', 'package box 包 打包 归档'),
  ('🎁', 'gift 礼物'),
  ('🏷️', 'label tag 标签'),
  ('🔖', 'bookmark 书签'),
  ('📊', 'bar chart 图表 数据 统计'),
  ('📈', 'chart up 增长 上升 数据'),
  ('📉', 'chart down 下降 数据'),
  ('🧾', 'receipt 收据 账单'),
  ('💰', 'money bag 钱 预算'),
  ('💳', 'credit card 信用卡 支付'),
  ('🏦', 'bank 银行'),
  ('🏠', 'house home 家 首页'),
  ('🏢', 'office building 公司 办公'),
  ('🏭', 'factory 工厂'),
  ('🚀', 'rocket launch 火箭 发布 上线'),
  ('✈️', 'airplane 飞机 出行'),
  ('🚗', 'car 汽车'),
  ('🚲', 'bicycle 自行车'),
  ('🛰️', 'satellite 卫星'),
  ('🧭', 'compass 指南针 方向'),
  ('🗺️', 'map 地图 规划'),
  ('🎯', 'target dart 目标 靶'),
  ('🏆', 'trophy 奖杯 成就'),
  ('🥇', 'gold medal 第一 金牌'),
  ('🎖️', 'medal 勋章'),
  ('🎓', 'graduation cap 毕业 学习'),
  ('🎃', 'pumpkin 南瓜 万圣节'),
  ('🎉', 'party popper 庆祝 完成'),
  ('🎊', 'confetti 庆祝'),
  ('🎈', 'balloon 气球'),
  ('🕯️', 'candle 蜡烛'),
  ('🛡️', 'shield 盾 安全 防护'),
  ('⚖️', 'scales balance 天平 权衡'),
  ('🔗', 'link chain 链接 关联'),
  ('🧩', 'puzzle piece 拼图 模块'),
  ('🎮', 'game controller 游戏'),
  ('🎧', 'headphones 耳机 音乐'),
  ('🎬', 'clapper film 电影 视频'),
  ('📷', 'camera photo 相机 照片'),
  ('📺', 'television 电视'),
  ('🎤', 'microphone 麦克风 播客'),
  ('🎵', 'music note 音乐'),
  ('🖼️', 'framed picture 图片 画'),
  ('✉️', 'envelope mail 邮件 信'),
  ('📮', 'postbox 邮箱'),
  ('☎️', 'telephone 电话'),
  ('🔋', 'battery 电池 电量'),
  ('🔌', 'plug 插头 电源'),
  ('🧲', 'magnet 磁铁 吸引'),
  ('🪝', 'hook 钩子'),
  ('🧵', 'thread 线 线程'),
  ('🪟', 'window 窗 窗口'),
  ('🚪', 'door 门 入口'),
];

const List<_E> _kSymbols = [
  ('✅', 'check done 完成 对 勾'),
  ('☑️', 'checkbox ticked 勾选 待办'),
  ('✔️', 'check mark 对 勾'),
  ('❌', 'cross wrong 错 取消 失败'),
  ('⭕', 'circle right 圈 正确'),
  ('❗', 'exclamation 重要 注意'),
  ('❓', 'question 问题 疑问'),
  ('⚠️', 'warning caution 警告 注意'),
  ('🚫', 'prohibited 禁止 禁用'),
  ('⛔', 'no entry 禁止'),
  ('🔴', 'red circle 红 停 严重'),
  ('🟠', 'orange circle 橙 中等'),
  ('🟡', 'yellow circle 黄 提醒'),
  ('🟢', 'green circle 绿 正常 通过'),
  ('🔵', 'blue circle 蓝'),
  ('🟣', 'purple circle 紫'),
  ('⚫', 'black circle 黑'),
  ('⚪', 'white circle 白'),
  ('🟥', 'red square 红块'),
  ('🟧', 'orange square 橙块'),
  ('🟨', 'yellow square 黄块'),
  ('🟩', 'green square 绿块'),
  ('🟦', 'blue square 蓝块'),
  ('🟪', 'purple square 紫块'),
  ('⬛', 'black square 黑块'),
  ('⬜', 'white square 白块'),
  ('🔺', 'red triangle up 三角 上升'),
  ('🔻', 'red triangle down 三角 下降'),
  ('🔶', 'orange diamond 菱形'),
  ('🔷', 'blue diamond 菱形'),
  ('💠', 'diamond dot 菱形 装饰'),
  ('♻️', 'recycle 回收 复用'),
  ('♾️', 'infinity 无限'),
  ('▶️', 'play start 播放 开始'),
  ('⏸️', 'pause 暂停'),
  ('⏹️', 'stop 停止'),
  ('⏩', 'fast forward 快进 加速'),
  ('🔄', 'refresh sync 刷新 同步'),
  ('🔁', 'repeat loop 循环 重复'),
  ('🔀', 'shuffle 随机 乱序'),
  ('➕', 'plus add 加 新增'),
  ('➖', 'minus 减 移除'),
  ('✖️', 'multiply 乘 叉'),
  ('➗', 'divide 除'),
  ('🟰', 'equals 等于'),
  ('🔢', 'numbers 数字 序号'),
  ('🔤', 'letters abc 字母'),
  ('💬', 'speech bubble 评论 对话 消息'),
  ('💭', 'thought bubble 想法 思考'),
  ('❤️', 'red heart 心 爱 喜欢'),
  ('🧡', 'orange heart 心 橙'),
  ('💛', 'yellow heart 心 黄'),
  ('💚', 'green heart 心 绿'),
  ('💙', 'blue heart 心 蓝'),
  ('💜', 'purple heart 心 紫'),
  ('🖤', 'black heart 心 黑'),
  ('💯', 'hundred points 满分 一百'),
  ('🆕', 'new 新 新建'),
  ('🆙', 'up level 提升 升级'),
  ('🔝', 'top 置顶 最上'),
];

const List<_E> _kFlags = [
  ('🚩', 'red flag 旗 标记 重要'),
  ('🏁', 'chequered flag 终点 完成 比赛'),
  ('🏳️', 'white flag 白旗 投降'),
  ('🏴', 'black flag 黑旗'),
  ('🎌', 'crossed flags 旗 交叉'),
  ('🏴‍☠️', 'pirate flag 海盗旗'),
  ('⛳', 'golf flag 高尔夫 目标'),
  ('🎏', 'carp streamer 鲤鱼旗'),
];
