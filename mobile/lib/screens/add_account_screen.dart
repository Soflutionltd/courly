import 'package:courly_mobile/screens/password_account_screen.dart';
import 'package:courly_mobile/src/rust/api/accounts.dart';
import 'package:courly_mobile/src/rust/api/microsoft.dart';
import 'package:courly_mobile/widgets/glass.dart';
import 'package:flutter/services.dart';
import 'package:flutter_web_auth_2/flutter_web_auth_2.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:material_ui/material_ui.dart';

/// The sheet the "+" opens, as in Outlook: add a mail account, or cancel.
/// Resolves to `true` once an account was added, so the caller reloads.
Future<bool?> showAddAccountSheet(BuildContext context) async {
  final wantsMail = await showModalBottomSheet<bool>(
    context: context,
    showDragHandle: true,
    builder: (sheetContext) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ListTile(
            key: const Key('sheet-add-mail'),
            leading: const Icon(LucideIcons.mail),
            title: const Text('Ajouter un compte de courrier'),
            subtitle: const Text('Outlook, Gmail, Exchange, iCloud…'),
            onTap: () => Navigator.of(sheetContext).pop(true),
          ),
          const Divider(height: 1),
          ListTile(
            key: const Key('sheet-cancel'),
            title: const Text('Annuler', textAlign: TextAlign.center),
            onTap: () => Navigator.of(sheetContext).pop(false),
          ),
        ],
      ),
    ),
  );
  if (wantsMail != true || !context.mounted) return false;
  return Navigator.of(context).push<bool>(
    MaterialPageRoute(
      fullscreenDialog: true,
      builder: (_) => const AddAccountScreen(),
    ),
  );
}

/// "Ajouter un compte": the address alone, then Courly finds how it signs
/// in. Closing with the X returns to the app with nothing added.
class AddAccountScreen extends StatefulWidget {
  const AddAccountScreen({super.key});

  @override
  State<AddAccountScreen> createState() => _AddAccountScreenState();
}

class _AddAccountScreenState extends State<AddAccountScreen> {
  static const _domains = [
    'outlook.com',
    'gmail.com',
    'icloud.com',
    'hotmail.com',
    'yahoo.com',
  ];

  final _email = TextEditingController();
  bool _isWorking = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    _email.addListener(() => setState(() {}));
  }

  @override
  void dispose() {
    _email.dispose();
    super.dispose();
  }

  bool get _looksLikeAddress {
    final text = _email.text.trim();
    final at = text.indexOf('@');
    return at > 0 && at < text.length - 1 && text.substring(at).contains('.');
  }

  void _useDomain(String domain) {
    final text = _email.text.trim();
    final at = text.indexOf('@');
    final local = at < 0 ? text : text.substring(0, at);
    final value = '$local@$domain';
    _email.value = TextEditingValue(
      text: value,
      selection: TextSelection.collapsed(offset: local.length),
    );
  }

  Future<void> _continue() async {
    final email = _email.text.trim();
    setState(() {
      _isWorking = true;
      _error = null;
    });
    try {
      final found = await discoverAccount(email: email);
      if (!mounted) return;
      switch (found.method) {
        case SignInMethod.microsoft:
          await _signInWithMicrosoft(email);
        case SignInMethod.imap:
        case SignInMethod.manual:
          setState(() => _isWorking = false);
          final added = await Navigator.of(context).push<bool>(
            MaterialPageRoute(
              builder: (_) =>
                  PasswordAccountScreen(email: email, discovered: found),
            ),
          );
          if (added == true && mounted) Navigator.of(context).pop(true);
      }
    } catch (error) {
      if (!mounted) return;
      setState(() => _error = 'Impossible de vérifier cette adresse : $error');
    } finally {
      if (mounted) setState(() => _isWorking = false);
    }
  }

  /// Microsoft's own sign-in page in Apple's authentication session, aimed
  /// at the typed address. Cancelling there leaves this screen as it was.
  Future<void> _signInWithMicrosoft(String email) async {
    final start = await microsoftSignInStart(emailHint: email);
    final String callback;
    try {
      callback = await FlutterWebAuth2.authenticate(
        url: start.authUrl,
        callbackUrlScheme: start.callbackScheme,
      );
    } on PlatformException catch (error) {
      if (error.code == 'CANCELED') return;
      rethrow;
    }
    await microsoftSignInFinish(callbackUrl: callback);
    if (mounted) Navigator.of(context).pop(true);
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return GlassPage(
      title: 'Ajouter un compte',
      leading: GlassIconButton(
        key: const Key('close-add-account'),
        symbol: 'xmark',
        icon: LucideIcons.x,
        label: 'Fermer',
        onPressed: () => Navigator.of(context).pop(false),
      ),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.all(24),
          children: [
            Text(
              'Entrez votre adresse de courrier professionnelle ou personnelle.',
              style: theme.textTheme.bodyLarge,
            ),
            const SizedBox(height: 24),
            TextField(
              key: const Key('email-field'),
              controller: _email,
              enabled: !_isWorking,
              autofocus: true,
              autocorrect: false,
              keyboardType: TextInputType.emailAddress,
              textInputAction: TextInputAction.go,
              autofillHints: const [AutofillHints.email],
              decoration: const InputDecoration(
                labelText: 'Adresse de courrier',
              ),
              onSubmitted: (_) {
                if (_looksLikeAddress && !_isWorking) _continue();
              },
            ),
            const SizedBox(height: 12),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                for (final domain in _domains)
                  ActionChip(
                    label: Text('@$domain'),
                    onPressed: _isWorking ? null : () => _useDomain(domain),
                  ),
              ],
            ),
            if (_error != null) ...[
              const SizedBox(height: 16),
              Text(_error!, style: TextStyle(color: theme.colorScheme.error)),
            ],
            const SizedBox(height: 32),
            FilledButton(
              key: const Key('add-account-continue'),
              onPressed: _looksLikeAddress && !_isWorking ? _continue : null,
              child: _isWorking
                  ? const SizedBox.square(
                      dimension: 20,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  : const Text('Ajouter un compte'),
            ),
          ],
        ),
      ),
    );
  }
}
