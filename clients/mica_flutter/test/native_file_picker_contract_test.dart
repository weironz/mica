import 'dart:convert';
import 'dart:typed_data';

import 'package:file_picker/file_picker.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/image_actions_stub.dart';
import 'package:mica_flutter/editor/pick_file_stub.dart';
import 'package:mica_flutter/editor/pick_image_stub.dart';

final class _PickedFile extends PlatformFile {
  _PickedFile(this.name, this.data, {this.unreadable = false});
  @override
  final String name;
  final Uint8List data;
  final bool unreadable;
  @override
  Uri get uri => Uri.parse('content://test-provider/$name');
  @override
  Never get xFile => throw UnimplementedError();
  @override
  int lengthSync() => data.length;
  @override
  Future<int> length() async => data.length;
  @override
  Future<Uint8List> readAsBytes() async {
    if (unreadable) throw StateError('unreadable fixture');
    return data;
  }

  @override
  Stream<Uint8List> readAsByteStream() => Stream.value(data);
}

class _Picker extends FilePickerPlatform {
  PlatformFile? selected;
  List<PlatformFile> multiple = [];
  Uint8List? savedBytes;
  String? savedMime;
  @override
  Future<PlatformFile?> pickFile({
    String? dialogTitle,
    String? initialDirectory,
    FileType type = FileType.any,
    List<String>? allowedExtensions,
    Function(FilePickerStatus)? onFileLoading,
    int compressionQuality = 0,
    AndroidOptions androidOptions = const AndroidOptions(),
    DarwinOptions darwinOptions = const DarwinOptions(),
    WindowsOptions windowsOptions = const WindowsOptions(),
    LinuxOptions linuxOptions = const LinuxOptions(),
    WebOptions webOptions = const WebOptions(),
  }) async => selected;
  @override
  Future<List<PlatformFile>> pickFiles({
    String? dialogTitle,
    String? initialDirectory,
    FileType type = FileType.any,
    List<String>? allowedExtensions,
    Function(FilePickerStatus)? onFileLoading,
    int compressionQuality = 0,
    AndroidOptions androidOptions = const AndroidOptions(),
    DarwinOptions darwinOptions = const DarwinOptions(),
    WindowsOptions windowsOptions = const WindowsOptions(),
    LinuxOptions linuxOptions = const LinuxOptions(),
    WebOptions webOptions = const WebOptions(),
  }) async => multiple;
  @override
  Future<Uri?> saveFile({
    required String fileName,
    required Uint8List bytes,
    String mimeType = 'application/octet-stream',
    String? dialogTitle,
    String? initialDirectory,
    Function(FilePickerStatus)? onFileSaving,
    WindowsOptions windowsOptions = const WindowsOptions(),
    LinuxOptions linuxOptions = const LinuxOptions(),
    WebOptions webOptions = const WebOptions(),
  }) async {
    savedBytes = bytes;
    savedMime = mimeType;
    return Uri.parse('content://test-provider/saved');
  }
}

void main() {
  late FilePickerPlatform original;
  late _Picker picker;
  setUp(() {
    original = FilePickerPlatform.instance;
    picker = _Picker();
    FilePickerPlatform.instance = picker;
  });
  tearDown(() => FilePickerPlatform.instance = original);
  test(
    'cancelled single and multiple picks preserve the empty contracts',
    () async {
      expect(await pickTextFile(), isNull);
      expect(await pickImage(), isNull);
      expect(await pickImportFiles(), isEmpty);
    },
  );
  test(
    'text and image use the provider bytes without requiring a local path',
    () async {
      picker.selected = _PickedFile(
        'note.md',
        Uint8List.fromList(utf8.encode('中文 text')),
      );
      expect(await pickTextFile(), (name: 'note.md', text: '中文 text'));
      final bytes = Uint8List.fromList([1, 2, 3]);
      picker.selected = _PickedFile('picture.png', bytes);
      final image = await pickImage();
      expect(image?.mime, 'image/png');
      expect(image?.bytes, bytes);
    },
  );
  test(
    'unreadable entries are rejected without losing other selected files',
    () async {
      final bytes = Uint8List.fromList([1]);
      picker.selected = _PickedFile('bad.md', bytes, unreadable: true);
      expect(await pickTextFile(), isNull);
      picker.multiple = [picker.selected!, _PickedFile('good.md', bytes)];
      expect((await pickImportFiles()).map((file) => file.name), ['good.md']);
    },
  );
  test(
    'save forwards bytes and MIME and accepts the plugin content URI',
    () async {
      final bytes = Uint8List.fromList([1, 2, 3]);
      downloadImage(bytes, 'image.png', 'image/png');
      await Future<void>.delayed(Duration.zero);
      expect(picker.savedBytes, bytes);
      expect(picker.savedMime, 'image/png');
    },
  );
}
