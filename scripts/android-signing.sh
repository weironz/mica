# Stable release signing preflight. The Gradle signer remains the final check.
# No secret value is printed or placed on a command line.
require_android_signing() {
  local name value type
  for name in MICA_ANDROID_KEYSTORE_PATH MICA_ANDROID_KEY_ALIAS MICA_ANDROID_STORE_PASSWORD MICA_ANDROID_KEY_PASSWORD; do
    value=${!name:-}
    [ -n "${value//[[:space:]]/}" ] || fail "release signing requires $name (debug builds are unaffected)"
  done
  [ -f "$MICA_ANDROID_KEYSTORE_PATH" ] && [ -r "$MICA_ANDROID_KEYSTORE_PATH" ] \
    || fail "MICA_ANDROID_KEYSTORE_PATH must name a readable keystore file"
  type=${MICA_ANDROID_STORE_TYPE:-JKS}
  case "${type^^}" in JKS|PKCS12) ;; *) fail "MICA_ANDROID_STORE_TYPE must be JKS or PKCS12" ;; esac
}
