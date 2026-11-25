import 'package:courly_mobile/main.dart';
import 'package:courly_mobile/src/rust/frb_generated.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => await RustLib.init());

  testWidgets('the Rust core opens the Courly database on the device', (
    tester,
  ) async {
    await tester.pumpWidget(const CourlyApp());
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('database-error')), findsNothing);
    expect(find.byKey(const Key('database-report')), findsOneWidget);
    expect(find.textContaining('Courly core ready'), findsOneWidget);
  });
}
