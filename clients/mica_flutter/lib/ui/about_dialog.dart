import 'package:flutter/material.dart';

import '../editor/open_url.dart';
import '../l10n/locale_controller.dart';
import '../widgets/mica_logo.dart';
import 'update_controls.dart';

/// The settings and application menus open the same version/update details.
void showMicaAboutDialog(
  BuildContext context,
  DesktopUpdateController updates,
) => showAboutDialog(
  context: context,
  applicationName: 'Mica',
  applicationVersion: 'v${updates.currentVersion}',
  applicationIcon: const MicaLogo(size: 40),
  applicationLegalese: context.l10n.aboutLegalese,
  children: [
    const SizedBox(height: 16),
    Align(
      alignment: Alignment.centerLeft,
      child: TextButton.icon(
        onPressed: () => openUrl('https://github.com/weironz/mica'),
        icon: const Icon(Icons.code, size: 16),
        label: Text(context.l10n.aboutSourceCode),
      ),
    ),
    if (updates.supported) ...[
      const SizedBox(height: 8),
      UpdateChecker(controller: updates),
    ],
  ],
);
