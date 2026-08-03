/// At-rest protection for the secrets in `prefs.json` — on Windows, DPAPI.
///
/// The session token and refresh token used to sit in that file in plaintext, so
/// anything that read a backup, a synced profile or a support bundle could take
/// the account. DPAPI ties the ciphertext to the Windows user account: copying
/// `prefs.json` to another machine, or reading it as another user, yields
/// nothing.
///
/// **What this is not.** DPAPI does not defend against code running AS the user
/// — that code can call `CryptUnprotectData` too. Nothing on a desktop can; the
/// OS keychain story is the same. What it removes is the *offline* copy: a file
/// that is readable anywhere it lands.
///
/// Bound directly with `dart:ffi` rather than adding a secure-storage package —
/// two calls and a struct, next to the Win32 binding this project already keeps
/// in-house (`window_snapped_win.dart`), and the package it would replace pulls
/// a plugin on every platform to solve a problem we have on one.
///
/// **Non-Windows desktop degrades to plaintext, honestly.** Linux has no
/// equivalent that works without a session keyring daemon, and `linux/` is not
/// even built in CI today. [secretsAreEncrypted] says which world you are in, so
/// nothing has to guess — and so "protected" never quietly means "not".
library;

import 'dart:convert';
import 'dart:ffi';
import 'dart:io' show Platform;
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

/// `DATA_BLOB` — the in/out shape of both DPAPI calls.
final class _DataBlob extends Struct {
  @Uint32()
  external int cbData;
  external Pointer<Uint8> pbData;
}

typedef _CryptProtectC =
    Int32 Function(
      Pointer<_DataBlob> dataIn,
      Pointer<Utf16> description,
      Pointer<_DataBlob> entropy,
      Pointer<Void> reserved,
      Pointer<Void> prompt,
      Uint32 flags,
      Pointer<_DataBlob> dataOut,
    );
typedef _CryptProtectDart =
    int Function(
      Pointer<_DataBlob> dataIn,
      Pointer<Utf16> description,
      Pointer<_DataBlob> entropy,
      Pointer<Void> reserved,
      Pointer<Void> prompt,
      int flags,
      Pointer<_DataBlob> dataOut,
    );

typedef _LocalFreeC = Pointer<Void> Function(Pointer<Void> mem);
typedef _LocalFreeDart = Pointer<Void> Function(Pointer<Void> mem);

/// Never show UI. A token refresh runs in the background; a modal prompt there
/// would be an unexplained dialog at best and a hang at worst.
const int _kCryptprotectUiForbidden = 0x1;

/// Marks a value this file produced. Without it there is no way to tell
/// ciphertext from a legacy plaintext token, and the migration would have to
/// guess — see [unprotect], which reads the absence of this prefix as "written
/// before encryption existed".
const String _kPrefix = 'dpapi1:';

/// Whether values handed to [protect] actually come back encrypted.
bool get secretsAreEncrypted => Platform.isWindows && _crypt32() != null;

DynamicLibrary? _crypt32Cache;
bool _crypt32Tried = false;

DynamicLibrary? _crypt32() {
  if (_crypt32Tried) return _crypt32Cache;
  _crypt32Tried = true;
  if (!Platform.isWindows) return _crypt32Cache = null;
  try {
    final lib = DynamicLibrary.open('crypt32.dll');
    if (!lib.providesSymbol('CryptProtectData') ||
        !lib.providesSymbol('CryptUnprotectData')) {
      return _crypt32Cache = null;
    }
    return _crypt32Cache = lib;
  } catch (_) {
    return _crypt32Cache = null;
  }
}

/// Encrypt [value] for this Windows user. Returns it unchanged where DPAPI is
/// unavailable — a caller must not be able to tell "protected" from "silently
/// not protected" by the return value, so [secretsAreEncrypted] is the question
/// to ask instead.
String protect(String value) {
  if (value.isEmpty) return value;
  // IDEMPOTENT, and that is load-bearing. The migration in `prefs_stub` runs on
  // every cold start and walks every secret key, so without this it would
  // re-encrypt already-encrypted values on the second launch: `unprotect` then
  // peels one layer and returns a `dpapi1:` STRING, which goes out as an
  // `Authorization` header and fails in a way that looks nothing like its cause.
  // (A real token cannot collide with the prefix — JWTs are base64url and start
  // `eyJ` — so treating it as "already done" is safe.)
  if (value.startsWith(_kPrefix)) return value;
  final bytes = _run(const Utf8Encoder().convert(value), encrypt: true);
  if (bytes == null) return value;
  return '$_kPrefix${base64Encode(bytes)}';
}

/// Decrypt a value produced by [protect].
///
/// Three outcomes, deliberately different:
/// * no prefix → a plaintext token written before this existed. Returned as-is,
///   so signing in again is not the price of the upgrade; the caller re-saves
///   it, which encrypts it.
/// * prefix, decrypts → the secret.
/// * prefix, will not decrypt → **null**, meaning "no token", not the raw
///   ciphertext. That is the copied-profile / different-user case, and the right
///   answer there is the sign-in screen. Handing back bytes that are not a token
///   would send a garbage `Authorization` header instead.
String? unprotect(String stored) {
  if (!stored.startsWith(_kPrefix)) return stored;
  final Uint8List cipher;
  try {
    cipher = base64Decode(stored.substring(_kPrefix.length));
  } catch (_) {
    return null;
  }
  final plain = _run(cipher, encrypt: false);
  if (plain == null) return null;
  try {
    return const Utf8Decoder().convert(plain);
  } catch (_) {
    return null;
  }
}

/// The two DPAPI calls differ only by symbol name, so they share a body — the
/// allocate/free-on-every-path dance is the part worth writing once.
Uint8List? _run(Uint8List input, {required bool encrypt}) {
  final lib = _crypt32();
  if (lib == null) return null;

  final Pointer<_DataBlob> inBlob = calloc<_DataBlob>();
  final Pointer<_DataBlob> outBlob = calloc<_DataBlob>();
  final Pointer<Uint8> buffer = calloc<Uint8>(input.length);
  try {
    final fn = lib.lookupFunction<_CryptProtectC, _CryptProtectDart>(
      encrypt ? 'CryptProtectData' : 'CryptUnprotectData',
    );
    final localFree = DynamicLibrary.open(
      'kernel32.dll',
    ).lookupFunction<_LocalFreeC, _LocalFreeDart>('LocalFree');

    buffer.asTypedList(input.length).setAll(0, input);
    inBlob.ref
      ..cbData = input.length
      ..pbData = buffer;

    final ok = fn(
      inBlob,
      nullptr,
      nullptr,
      nullptr,
      nullptr,
      _kCryptprotectUiForbidden,
      outBlob,
    );
    if (ok == 0) return null;

    // Copy before freeing: the returned buffer is DPAPI's, and holding a view
    // into it past LocalFree is a use-after-free that would surface as corrupted
    // tokens long after the call that caused it.
    final result = Uint8List.fromList(
      outBlob.ref.pbData.asTypedList(outBlob.ref.cbData),
    );
    localFree(outBlob.ref.pbData.cast<Void>());
    return result;
  } catch (_) {
    return null;
  } finally {
    calloc.free(buffer);
    calloc.free(inBlob);
    calloc.free(outBlob);
  }
}
