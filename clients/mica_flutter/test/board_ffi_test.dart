import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/board/board_bridge.dart';
import 'package:mica_flutter/board/board_models.dart';
import 'package:mica_flutter/src/rust/api/board.dart' as rust_board;

import 'support/native_build.dart';

void main() {
  setUpAll(() async {
    await initCheckedNativeRust();
  });

  test('Rust FFI board engine: SVG, mindmap, routing, merge, and draw.io import', () async {
    // 1. Starter template -> headless SVG
    final starter = await rust_board.starterBoardJson(template: 'flowchart');
    final svg = await boardRenderSvg(
      starter,
      theme: const BoardThemeInput(
        dark: false,
        canvas: '#F7F8FA',
        surface: '#FFFFFF',
        text: '#1F2329',
        subtleText: '#646A73',
        border: '#8C959F',
        accent: '#3370FF',
      ),
    );
    expect(svg, isNotNull);
    expect(svg!, startsWith('<svg '));
    expect(svg, contains('校验通过？'));

    // 2. Orthogonal edge routing via Rust
    final routedRaw = await boardRouteEdgesJson(starter);
    expect(routedRaw, isNotNull);
    final routed = jsonDecode(routedRaw!) as Map<String, dynamic>;
    expect(routed.keys, contains('fe1'));

    // 3. Markdown outline <-> Mind map round-trip
    final mindJson = await boardMarkdownToMindmap(
      '# 产品规划\n- 画板引擎\n  - 图形库\n  - draw.io 导入\n- 协同\n',
      title: '产品规划',
    );
    expect(mindJson, isNotNull);
    final mdBack = await boardMindmapToMarkdown(mindJson!);
    expect(mdBack, isNotNull);
    expect(mdBack!, contains('# 产品规划'));
    expect(mdBack, contains('draw.io 导入'));

    // 4. draw.io XML import via Rust FFI
    const sampleDrawio =
        '<mxfile><diagram name="网关架构"><mxGraphModel><root>'
        '<mxCell id="0"/><mxCell id="1" parent="0"/>'
        '<mxCell id="a" value="Nginx" style="rounded=1;" vertex="1" parent="1">'
        '<mxGeometry x="20" y="20" width="120" height="50" as="geometry"/></mxCell>'
        '<mxCell id="b" value="Postgres" style="shape=cylinder3;" vertex="1" parent="1">'
        '<mxGeometry x="240" y="20" width="120" height="70" as="geometry"/></mxCell>'
        '<mxCell id="e" value="TCP" style="edgeStyle=orthogonalEdgeStyle;" edge="1" parent="1" source="a" target="b">'
        '<mxGeometry relative="1" as="geometry"/></mxCell>'
        '</root></mxGraphModel></diagram></mxfile>';
    final imported = await boardImportDrawio(utf8.encode(sampleDrawio));
    expect(imported, isNotNull);
    final snap = BoardSnapshot.tryParseJson(imported!);
    expect(snap, isNotNull);
    expect(snap!.title, '网关架构');
    expect(snap.nodes.length, 2);
    expect(snap.nodes[1].kind, 'cylinder');
    expect(snap.edges.length, 1);
    expect(snap.edges[0].label, 'TCP');

    // 5. 3-way semantic merge via Rust FFI
    final base = BoardSnapshot.starter('whiteboard');
    final local = base.deepCopy()..nodes[0].x = 280;
    final remote = base.deepCopy()..nodes[0].text = '远端修改标题';
    final mergedRaw = await boardMergeJson(
      base.toCompactJson(),
      local.toCompactJson(),
      remote.toCompactJson(),
    );
    final merged = BoardSnapshot.tryParseJson(mergedRaw)!;
    expect(merged.nodes[0].x, closeTo(280, 0.1));
    expect(merged.nodes[0].text, '远端修改标题');

    // 6. Plain-text extraction for search
    final plain = await rust_board.extractBoardPlainText(source: mergedRaw);
    expect(plain, contains('远端修改标题'));
  });
}
