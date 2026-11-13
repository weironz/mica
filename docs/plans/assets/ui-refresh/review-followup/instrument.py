"""Apply identical temporary timing overlays to each isolated source snapshot."""
from pathlib import Path
import hashlib
import json
import sys

root = Path(sys.argv[1])
out = Path(sys.argv[2])
helper = Path(__file__).with_name('startup_probe.dart').read_text()
manifest = {}
for name in ['lib/main.dart', 'lib/ui/font_warmup.dart']:
    target = root / name
    original = target.read_text(encoding='utf-8-sig')
    manifest[name] = {'originalSha256': hashlib.sha256(original.encode()).hexdigest()}
    if name.endswith('main.dart'):
        assert original.count('void main() {') == 1
        assert original.count('await warmUpFonts();') == 1
        assert original.count('runApp(const MicaApp());') == 1
        result = "import 'ui/startup_probe.dart' as startup_probe;\n" + original
        result = result.replace('void main() {', 'void main() {\n  startup_probe.begin();')
        result = result.replace('await warmUpFonts();', "await startup_probe.measure('warmUpFonts', warmUpFonts);")
        result = result.replace('runApp(const MicaApp());', 'runApp(const MicaApp());\n      startup_probe.firstFrameReceipt();')
    else:
        needle = 'await (FontLoader(family)..addFont(rootBundle.load(path))).load();'
        assert original.count(needle) == 1
        result = "import 'startup_probe.dart' as startup_probe;\n" + original
        result = result.replace(needle, "await startup_probe.measure('fontAsset:$family', () =>\n      (FontLoader(family)..addFont(rootBundle.load(path))).load());")
    manifest[name]['instrumentedSha256'] = hashlib.sha256(result.encode()).hexdigest()
    target.write_text(result, encoding='utf-8', newline='\n')
(root / 'lib/ui/startup_probe.dart').write_text(helper, encoding='utf-8', newline='\n')
out.write_text(json.dumps(manifest, indent=2), encoding='utf-8')
