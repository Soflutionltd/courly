import 'package:courly_mobile/src/rust/api/accounts.dart';
import 'package:material_ui/material_ui.dart';
import 'package:url_launcher/url_launcher.dart';

/// The password step for IMAP mailboxes. For Gmail it explains the app
/// password Google requires and links to the page that makes one. The
/// servers discovery found are shown folded and stay editable; when
/// nothing was found they are open and required.
class PasswordAccountScreen extends StatefulWidget {
  const PasswordAccountScreen({
    super.key,
    required this.email,
    required this.discovered,
  });

  final String email;
  final DiscoveredAccount discovered;

  @override
  State<PasswordAccountScreen> createState() => _PasswordAccountScreenState();
}

class _PasswordAccountScreenState extends State<PasswordAccountScreen> {
  static const _securities = ['tls', 'starttls', 'none'];

  final _password = TextEditingController();
  late final TextEditingController _imapHost;
  late final TextEditingController _imapPort;
  late final TextEditingController _smtpHost;
  late final TextEditingController _smtpPort;
  late final TextEditingController _username;
  late String _imapSecurity;
  late String _smtpSecurity;
  bool _isWorking = false;
  bool _isPasswordHidden = true;
  String? _error;

  @override
  void initState() {
    super.initState();
    final servers = widget.discovered.servers;
    _imapHost = TextEditingController(text: servers?.imapHost ?? '');
    _imapPort = TextEditingController(text: '${servers?.imapPort ?? 993}');
    _smtpHost = TextEditingController(text: servers?.smtpHost ?? '');
    _smtpPort = TextEditingController(text: '${servers?.smtpPort ?? 465}');
    _username = TextEditingController(text: servers?.username ?? widget.email);
    _imapSecurity = servers?.imapSecurity ?? 'tls';
    _smtpSecurity = servers?.smtpSecurity ?? 'tls';
    for (final controller in [
      _password,
      _imapHost,
      _imapPort,
      _smtpHost,
      _smtpPort,
      _username,
    ]) {
      controller.addListener(() => setState(() {}));
    }
  }

  @override
  void dispose() {
    for (final controller in [
      _password,
      _imapHost,
      _imapPort,
      _smtpHost,
      _smtpPort,
      _username,
    ]) {
      controller.dispose();
    }
    super.dispose();
  }

  bool get _isGmail => widget.discovered.appPasswordUrl != null;

  ImapServers? get _servers {
    final imapPort = int.tryParse(_imapPort.text.trim());
    final smtpPort = int.tryParse(_smtpPort.text.trim());
    final imapHost = _imapHost.text.trim();
    final smtpHost = _smtpHost.text.trim();
    final username = _username.text.trim();
    if (imapPort == null || smtpPort == null) return null;
    if (imapHost.isEmpty || smtpHost.isEmpty || username.isEmpty) return null;
    return ImapServers(
      imapHost: imapHost,
      imapPort: imapPort,
      imapSecurity: _imapSecurity,
      smtpHost: smtpHost,
      smtpPort: smtpPort,
      smtpSecurity: _smtpSecurity,
      username: username,
    );
  }

  bool get _canAdd =>
      !_isWorking && _password.text.isNotEmpty && _servers != null;

  Future<void> _openAppPasswords() async {
    final url = widget.discovered.appPasswordUrl;
    if (url == null) return;
    final opened = await launchUrl(
      Uri.parse(url),
      mode: LaunchMode.externalApplication,
    );
    if (!opened && mounted) {
      setState(() => _error = 'La page Google n\'a pas pu s\'ouvrir : $url');
    }
  }

  Future<void> _add() async {
    final servers = _servers;
    if (servers == null) return;
    setState(() {
      _isWorking = true;
      _error = null;
    });
    try {
      final outcome = await addPasswordAccount(
        email: widget.email,
        password: _password.text,
        servers: servers,
      );
      if (!mounted) return;
      if (outcome.added != null) {
        Navigator.of(context).pop(true);
        return;
      }
      setState(() => _error = _failureText(outcome));
    } catch (error) {
      if (!mounted) return;
      setState(() => _error = 'Le compte n\'a pas pu être ajouté : $error');
    } finally {
      if (mounted) setState(() => _isWorking = false);
    }
  }

  String _failureText(AddPasswordResult outcome) {
    final detail = outcome.detail.isEmpty ? '' : '\n${outcome.detail}';
    return switch (outcome.failure) {
      AddFailure.wrongPassword =>
        _isGmail
            ? 'Google a refusé ce mot de passe. Utilisez un mot de passe '
                  'd\'application, pas celui de votre compte Google.$detail'
            : 'Le serveur a refusé l\'adresse ou le mot de passe.$detail',
      AddFailure.unreachable =>
        'Le serveur ne répond pas. Vérifiez les réglages du serveur ou '
            'votre connexion.$detail',
      AddFailure.other || null => 'Le compte n\'a pas pu être ajouté.$detail',
    };
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isManual = widget.discovered.method == SignInMethod.manual;
    return Scaffold(
      appBar: AppBar(title: Text(widget.email)),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.all(24),
          children: [
            if (_isGmail) ...[
              Card(
                child: Padding(
                  padding: const EdgeInsets.all(16),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        'Mot de passe d\'application Google',
                        style: theme.textTheme.titleMedium,
                      ),
                      const SizedBox(height: 8),
                      const Text(
                        'Gmail demande un mot de passe d\'application, et non '
                        'celui de votre compte. La validation en deux étapes '
                        'doit être activée sur votre compte Google. Créez-en '
                        'un, puis collez-le ci-dessous.',
                      ),
                      const SizedBox(height: 12),
                      OutlinedButton.icon(
                        key: const Key('open-app-passwords'),
                        onPressed: _openAppPasswords,
                        icon: const Icon(Icons.open_in_new),
                        label: const Text(
                          'Créer un mot de passe d\'application',
                        ),
                      ),
                    ],
                  ),
                ),
              ),
              const SizedBox(height: 24),
            ],
            TextField(
              key: const Key('password-field'),
              controller: _password,
              enabled: !_isWorking,
              autofocus: true,
              obscureText: _isPasswordHidden,
              autocorrect: false,
              enableSuggestions: false,
              autofillHints: const [AutofillHints.password],
              decoration: InputDecoration(
                labelText: _isGmail
                    ? 'Mot de passe d\'application'
                    : 'Mot de passe',
                border: const OutlineInputBorder(),
                suffixIcon: IconButton(
                  tooltip: _isPasswordHidden ? 'Afficher' : 'Masquer',
                  icon: Icon(
                    _isPasswordHidden ? Icons.visibility : Icons.visibility_off,
                  ),
                  onPressed: () =>
                      setState(() => _isPasswordHidden = !_isPasswordHidden),
                ),
              ),
            ),
            const SizedBox(height: 16),
            ExpansionTile(
              initiallyExpanded: isManual,
              tilePadding: EdgeInsets.zero,
              title: const Text('Réglages du serveur'),
              subtitle: isManual
                  ? const Text(
                      'Courly n\'a pas trouvé ce serveur : '
                      'saisissez ses réglages.',
                    )
                  : null,
              children: [
                _ServerFields(
                  label: 'Réception (IMAP)',
                  host: _imapHost,
                  port: _imapPort,
                  security: _imapSecurity,
                  securities: _securities,
                  enabled: !_isWorking,
                  onSecurity: (value) => setState(() => _imapSecurity = value),
                ),
                const SizedBox(height: 16),
                _ServerFields(
                  label: 'Envoi (SMTP)',
                  host: _smtpHost,
                  port: _smtpPort,
                  security: _smtpSecurity,
                  securities: _securities,
                  enabled: !_isWorking,
                  onSecurity: (value) => setState(() => _smtpSecurity = value),
                ),
                const SizedBox(height: 16),
                TextField(
                  controller: _username,
                  enabled: !_isWorking,
                  autocorrect: false,
                  decoration: const InputDecoration(
                    labelText: 'Identifiant',
                    border: OutlineInputBorder(),
                  ),
                ),
                const SizedBox(height: 8),
              ],
            ),
            if (_error != null) ...[
              const SizedBox(height: 16),
              Text(
                _error!,
                key: const Key('add-error'),
                style: TextStyle(color: theme.colorScheme.error),
              ),
            ],
            const SizedBox(height: 32),
            FilledButton(
              key: const Key('add-password-account'),
              onPressed: _canAdd ? _add : null,
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

class _ServerFields extends StatelessWidget {
  const _ServerFields({
    required this.label,
    required this.host,
    required this.port,
    required this.security,
    required this.securities,
    required this.enabled,
    required this.onSecurity,
  });

  final String label;
  final TextEditingController host;
  final TextEditingController port;
  final String security;
  final List<String> securities;
  final bool enabled;
  final ValueChanged<String> onSecurity;

  static String _securityLabel(String value) => switch (value) {
    'tls' => 'SSL/TLS',
    'starttls' => 'STARTTLS',
    _ => 'Aucune',
  };

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(label, style: Theme.of(context).textTheme.labelLarge),
        const SizedBox(height: 8),
        TextField(
          controller: host,
          enabled: enabled,
          autocorrect: false,
          keyboardType: TextInputType.url,
          decoration: const InputDecoration(
            labelText: 'Serveur',
            border: OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 8),
        Row(
          children: [
            SizedBox(
              width: 100,
              child: TextField(
                controller: port,
                enabled: enabled,
                keyboardType: TextInputType.number,
                decoration: const InputDecoration(
                  labelText: 'Port',
                  border: OutlineInputBorder(),
                ),
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: DropdownButtonFormField<String>(
                initialValue: security,
                decoration: const InputDecoration(
                  labelText: 'Sécurité',
                  border: OutlineInputBorder(),
                ),
                items: [
                  for (final value in securities)
                    DropdownMenuItem(
                      value: value,
                      child: Text(_securityLabel(value)),
                    ),
                ],
                onChanged: enabled
                    ? (value) {
                        if (value != null) onSecurity(value);
                      }
                    : null,
              ),
            ),
          ],
        ),
      ],
    );
  }
}
