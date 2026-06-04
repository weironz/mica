final Map<String, String> _memory = {};

String? loadPref(String key) => _memory[key];

void savePref(String key, String value) => _memory[key] = value;

void removePref(String key) => _memory.remove(key);
