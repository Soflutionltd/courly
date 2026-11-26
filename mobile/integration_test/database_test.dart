import 'package:courly_mobile/main.dart';
import 'package:courly_mobile/src/rust/frb_generated.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

/// Pumps until [finder] shows, for screens whose spinners never let
/// `pumpAndSettle` settle.
Future<void> pumpUntilFound(WidgetTester tester, Finder finder) async {
  for (var tick = 0; tick < 100; tick++) {
    await tester.pump(const Duration(milliseconds: 100));
    if (finder.evaluate().isNotEmpty) return;
  }
  throw TestFailure('$finder never appeared');
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => await RustLib.init());

  testWidgets('the engine boots, and adding an account can be cancelled', (
    tester,
  ) async {
    // The Liquid Glass buttons are native views: they are pressed through
    // their semantics tap, as VoiceOver does.
    final semantics = tester.ensureSemantics();
    await tester.pumpWidget(const CourlyApp());
    await pumpUntilFound(tester, find.byKey(const Key('open-menu')));
    expect(find.byKey(const Key('engine-error')), findsNothing);

    // Back from the inbox of every account to the mailboxes page.
    tester.semantics.tap(find.semantics.byLabel('Boîtes aux lettres'));
    await pumpUntilFound(tester, find.byKey(const Key('drawer-add-account')));
    // The page exists before the inbox has slid off it: let it finish.
    await tester.pump(const Duration(milliseconds: 600));
    await tester.tap(find.byKey(const Key('drawer-add-account')));
    await pumpUntilFound(tester, find.byKey(const Key('sheet-add-mail')));
    expect(find.text('Outlook, Gmail, Exchange, iCloud…'), findsOneWidget);

    await tester.tap(find.byKey(const Key('sheet-add-mail')));
    await pumpUntilFound(tester, find.byKey(const Key('email-field')));
    expect(
      find.text(
        'Entrez votre adresse de courrier professionnelle ou personnelle.',
      ),
      findsOneWidget,
    );

    tester.semantics.tap(find.semantics.byLabel('Fermer'));
    await pumpUntilFound(tester, find.byKey(const Key('drawer-add-account')));
    await tester.pump(const Duration(seconds: 1));
    expect(find.byKey(const Key('email-field')), findsNothing);
    semantics.dispose();
  });
}
