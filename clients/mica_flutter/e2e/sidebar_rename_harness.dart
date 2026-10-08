// Real workspace/sidebar/dialog, isolated synthetic documents and callbacks.
import 'dart:convert';
import 'dart:js_interop';
import 'package:flutter/material.dart';
import 'package:mica_flutter/main.dart';
import '../test/support/title_workspace.dart';

@JS('micaRenameState')
external set _readState(JSFunction value);

void main() => runApp(const _RenameHarness());

class _RenameHarness extends StatefulWidget {
  const _RenameHarness();
  @override
  State<_RenameHarness> createState() => _RenameHarnessState();
}

class _RenameHarnessState extends State<_RenameHarness> {
  final _current = titleBootstrap('当前页面');
  DocumentBootstrap _target = titleBootstrap('待重命名页面', id: 'page-b');
  var _selected = 'page-a';
  var _opens = 0;
  final _saves = <String>[];

  @override
  void initState() {
    super.initState();
    _readState = (() => jsonEncode({
      'opens': _opens,
      'saves': _saves,
      'name': _target.view.name,
    }).toJS).toJS;
  }

  @override
  Widget build(BuildContext context) => titleWorkspace(
    _selected == 'page-a' ? _current : _target,
    (view, name) async {
      if (view.id != 'page-b') throw StateError('Renamed the wrong page');
      setState(() {
        _saves.add(name);
        _target = titleBootstrap(name, id: 'page-b');
      });
      return true;
    },
    views: [_current.view, _target.view],
    onSelectView: (view) async => setState(() {
      _opens++;
      _selected = view.id;
    }),
  );
}
