import 'package:courly_mobile/src/rust/api/compose.dart';
import 'package:material_ui/material_ui.dart';
import 'package:webview_flutter/webview_flutter.dart';

/// Writes a new message or a reply. The reply arrives addressed and with
/// its subject, the original quoted under the text as Outlook does. Pops
/// with `true` once the engine has taken the mail.
class ComposeScreen extends StatefulWidget {
  const ComposeScreen({
    super.key,
    required this.dataDir,
    required this.mode,
    required this.accountId,
    this.replySource,
  });

  final String dataDir;
  final ComposeMode mode;
  final String accountId;
  final ReplySource? replySource;

  @override
  State<ComposeScreen> createState() => _ComposeScreenState();
}

class _ComposeScreenState extends State<ComposeScreen> {
  final _to = TextEditingController();
  final _cc = TextEditingController();
  final _bcc = TextEditingController();
  final _subject = TextEditingController();
  final _body = TextEditingController();

  ComposeDraft? _draft;
  String? _loadError;
  bool _showCopies = false;
  bool _sending = false;

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    for (final controller in [_to, _cc, _bcc, _subject, _body]) {
      controller.dispose();
    }
    super.dispose();
  }

  Future<void> _load() async {
    try {
      final draft = await composeDraft(
        dataDir: widget.dataDir,
        mode: widget.mode,
        accountId: widget.accountId,
        replySource: widget.replySource,
      );
      if (!mounted) return;
      setState(() {
        _draft = draft;
        _to.text = draft.to.join(', ');
        _cc.text = draft.cc.join(', ');
        _subject.text = draft.subject;
        _showCopies = draft.cc.isNotEmpty;
      });
    } catch (error) {
      if (!mounted) return;
      setState(() => _loadError = '$error');
    }
  }

  static List<String> _addresses(String field) => field
      .split(RegExp(r'[,;\s]+'))
      .where((address) => address.isNotEmpty)
      .toList();

  static final _address = RegExp(r'^[^@\s]+@[^@\s]+\.[^@\s]+$');

  bool get _dirty =>
      _body.text.trim().isNotEmpty ||
      (widget.mode == ComposeMode.new_ &&
          (_to.text.trim().isNotEmpty || _subject.text.trim().isNotEmpty));

  Future<void> _send() async {
    final draft = _draft;
    if (draft == null || _sending) return;
    final to = _addresses(_to.text);
    final cc = _addresses(_cc.text);
    final bcc = _addresses(_bcc.text);
    final all = [...to, ...cc, ...bcc];
    if (all.isEmpty) {
      _tell('Ajoutez au moins un destinataire.');
      return;
    }
    final invalid = all.where((address) => !_address.hasMatch(address));
    if (invalid.isNotEmpty) {
      _tell('Adresse invalide : ${invalid.first}');
      return;
    }
    setState(() => _sending = true);
    try {
      await sendMail(
        dataDir: widget.dataDir,
        mail: OutgoingMail(
          mode: widget.mode,
          accountId: draft.accountId,
          to: to,
          cc: cc,
          bcc: bcc,
          subject: _subject.text,
          bodyText: _body.text,
          replySource: widget.replySource,
        ),
      );
      if (!mounted) return;
      Navigator.of(context).pop(true);
    } catch (error) {
      if (!mounted) return;
      setState(() => _sending = false);
      _tell('Le message n\'a pas pu partir : $error');
    }
  }

  void _tell(String text) {
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));
  }

  Future<void> _close() async {
    if (!_dirty) {
      Navigator.of(context).pop(false);
      return;
    }
    final discard = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Abandonner ce message ?'),
        content: const Text('Le texte écrit sera perdu.'),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Continuer'),
          ),
          TextButton(
            key: const Key('discard-mail'),
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('Abandonner'),
          ),
        ],
      ),
    );
    if (discard == true && mounted) Navigator.of(context).pop(false);
  }

  String get _title => switch (widget.mode) {
    ComposeMode.new_ => 'Nouveau message',
    ComposeMode.reply => 'Répondre',
    ComposeMode.replyAll => 'Répondre à tous',
  };

  @override
  Widget build(BuildContext context) {
    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) _close();
      },
      child: Scaffold(
        appBar: AppBar(
          leading: IconButton(
            key: const Key('close-compose'),
            tooltip: 'Fermer',
            icon: const Icon(Icons.close),
            onPressed: _close,
          ),
          title: Text(_title),
          actions: [
            IconButton(
              key: const Key('send-mail'),
              tooltip: 'Envoyer',
              icon: _sending
                  ? const SizedBox(
                      width: 20,
                      height: 20,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  : const Icon(Icons.send),
              onPressed: _draft == null || _sending ? null : _send,
            ),
          ],
        ),
        body: _bodyView(context),
      ),
    );
  }

  Widget _bodyView(BuildContext context) {
    final error = _loadError;
    if (error != null) {
      return Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Text(
            'Le message n\'a pas pu être préparé : $error',
            textAlign: TextAlign.center,
          ),
        ),
      );
    }
    final draft = _draft;
    if (draft == null) {
      return const Center(child: CircularProgressIndicator());
    }
    final theme = Theme.of(context);
    return ListView(
      children: [
        _Line(
          label: 'De',
          child: Text(draft.fromEmail, style: theme.textTheme.bodyLarge),
        ),
        _Line(
          label: 'À',
          trailing: _showCopies
              ? null
              : IconButton(
                  key: const Key('show-copies'),
                  tooltip: 'Cc et Cci',
                  icon: const Icon(Icons.expand_more),
                  onPressed: () => setState(() => _showCopies = true),
                ),
          child: _Field(
            fieldKey: const Key('to-field'),
            controller: _to,
            keyboardType: TextInputType.emailAddress,
            autofocus: widget.mode == ComposeMode.new_,
          ),
        ),
        if (_showCopies) ...[
          _Line(
            label: 'Cc',
            child: _Field(
              fieldKey: const Key('cc-field'),
              controller: _cc,
              keyboardType: TextInputType.emailAddress,
            ),
          ),
          _Line(
            label: 'Cci',
            child: _Field(
              fieldKey: const Key('bcc-field'),
              controller: _bcc,
              keyboardType: TextInputType.emailAddress,
            ),
          ),
        ],
        _Line(
          label: 'Objet',
          child: _Field(
            fieldKey: const Key('subject-field'),
            controller: _subject,
            capitalization: TextCapitalization.sentences,
          ),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 16, 16),
          child: TextField(
            key: const Key('body-field'),
            controller: _body,
            autofocus: widget.mode != ComposeMode.new_,
            minLines: 8,
            maxLines: null,
            keyboardType: TextInputType.multiline,
            textCapitalization: TextCapitalization.sentences,
            decoration: const InputDecoration.collapsed(
              hintText: 'Votre message',
            ),
          ),
        ),
        if (draft.quotePreviewHtml.isNotEmpty)
          _QuotePreview(html: draft.quotePreviewHtml),
      ],
    );
  }
}

class _Line extends StatelessWidget {
  const _Line({required this.label, required this.child, this.trailing});

  final String label;
  final Widget child;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: theme.dividerColor)),
      ),
      child: Padding(
        padding: const EdgeInsets.only(left: 16, right: 4),
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 48),
          child: Row(
            children: [
              SizedBox(
                width: 52,
                child: Text(
                  label,
                  style: theme.textTheme.bodyMedium?.copyWith(
                    color: theme.colorScheme.onSurfaceVariant,
                  ),
                ),
              ),
              Expanded(child: child),
              ?trailing,
            ],
          ),
        ),
      ),
    );
  }
}

class _Field extends StatelessWidget {
  const _Field({
    required this.fieldKey,
    required this.controller,
    this.keyboardType,
    this.capitalization = TextCapitalization.none,
    this.autofocus = false,
  });

  final Key fieldKey;
  final TextEditingController controller;
  final TextInputType? keyboardType;
  final TextCapitalization capitalization;
  final bool autofocus;

  @override
  Widget build(BuildContext context) {
    return TextField(
      key: fieldKey,
      controller: controller,
      keyboardType: keyboardType,
      textCapitalization: capitalization,
      autofocus: autofocus,
      autocorrect: keyboardType != TextInputType.emailAddress,
      decoration: const InputDecoration.collapsed(hintText: ''),
    );
  }
}

/// The quoted original under the text, as the recipient will see it.
class _QuotePreview extends StatefulWidget {
  const _QuotePreview({required this.html});

  final String html;

  @override
  State<_QuotePreview> createState() => _QuotePreviewState();
}

class _QuotePreviewState extends State<_QuotePreview> {
  late final WebViewController _controller;
  double _height = 160;

  @override
  void initState() {
    super.initState();
    _controller = WebViewController()
      ..setJavaScriptMode(JavaScriptMode.disabled)
      ..setBackgroundColor(Colors.white)
      ..setNavigationDelegate(
        NavigationDelegate(
          onNavigationRequest: (request) => request.url.startsWith('about:')
              ? NavigationDecision.navigate
              : NavigationDecision.prevent,
          onPageFinished: (_) => _measure(),
        ),
      )
      ..loadHtmlString(widget.html);
  }

  Future<void> _measure() async {
    final result = await _controller.runJavaScriptReturningResult(
      'Math.ceil(document.documentElement.scrollHeight)',
    );
    final height = result is num
        ? result.toDouble()
        : double.tryParse('$result');
    if (height == null || !mounted) return;
    setState(() => _height = height);
  }

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: _height,
      child: WebViewWidget(controller: _controller),
    );
  }
}
