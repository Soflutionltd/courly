import 'package:courly_mobile/main.dart';
import 'package:courly_mobile/src/rust/frb_generated.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => await RustLib.init());

  testWidgets('the Courly engine boots on the device and offers sign-in', (
    tester,
  ) async {
    await tester.pumpWidget(const CourlyApp());
    await tester.pumpAndSettle(const Duration(seconds: 1));

    expect(find.byKey(const Key('engine-error')), findsNothing);
    expect(find.byKey(const Key('engine-ready')), findsOneWidget);
    expect(find.byKey(const Key('add-microsoft')), findsOneWidget);
  });
}
