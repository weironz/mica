import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/interface_text_scale.dart';

void main() {
  test('stored scales normalize to one of the supported choices', () {
    expect(uiFontScaleFromPref('0.9'), kUiFontScaleSmall);
    expect(uiFontScaleFromPref('1.08'), kUiFontScaleLarge);
    expect(uiFontScaleFromPref('1.04'), kUiFontScaleStandard);
    expect(uiFontScaleFromPref('NaN'), kUiFontScaleStandard);
    expect(uiFontScaleFromPref('bad'), kUiFontScaleStandard);
  });

  testWidgets('preference scales the platform text scaler live', (
    tester,
  ) async {
    uiFontScaleController.value = kUiFontScaleStandard;
    addTearDown(() {
      uiFontScaleController.value = kUiFontScaleStandard;
    });

    await tester.pumpWidget(
      MaterialApp(
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(
            context,
          ).copyWith(textScaler: const TextScaler.linear(1.2)),
          child: MicaInterfaceTextScale(child: child!),
        ),
        home: const Text('Scaled text', style: TextStyle(fontSize: 10)),
      ),
    );

    RichText renderedText() =>
        tester.widget<RichText>(find.byType(RichText).first);
    expect(renderedText().textScaler.scale(10), closeTo(12, 0.001));

    uiFontScaleController.value = kUiFontScaleLarge;
    await tester.pump();

    expect(renderedText().textScaler.scale(10), closeTo(13.2, 0.001));
    expect(tester.takeException(), isNull);
  });
}
