import 'package:courly_mobile/src/rust/api/thread.dart';
import 'package:material_ui/material_ui.dart';
import 'package:url_launcher/url_launcher.dart';
import 'package:webview_flutter/webview_flutter.dart';

/// One conversation, opened from the inbox: its messages oldest first, the
/// latest one open and the earlier ones folded to a line, as in Outlook.
class ThreadScreen extends StatefulWidget {
  const ThreadScreen({
    super.key,
    required this.dataDir,
    required this.accountId,
    required this.threadId,
    required this.markRead,
  });

  final String dataDir;
  final String accountId;
  final String threadId;

  /// Whether opening the conversation should mark it read.
  final bool markRead;

  @override
  State<ThreadScreen> createState() => _ThreadScreenState();
}

class _ThreadScreenState extends State<ThreadScreen> {
  ThreadView? _thread;
  String? _loadError;
  bool _loadRemoteImages = false;

  /// Messages the reader unfolded, on top of the latest one.
  final Set<String> _opened = {};

  @override
  void initState() {
    super.initState();
    _load();
    if (widget.markRead) _markRead();
  }

  Future<void> _load() async {
    try {
      final thread = await threadDetail(
        dataDir: widget.dataDir,
        accountId: widget.accountId,
        threadId: widget.threadId,
        loadRemoteImages: _loadRemoteImages,
      );
      if (!mounted) return;
      setState(() {
        _thread = thread;
        _loadError = null;
      });
    } catch (error) {
      if (!mounted) return;
      setState(() => _loadError = '$error');
    }
  }

  Future<void> _markRead() async {
    try {
      await markThreadRead(
        accountId: widget.accountId,
        threadId: widget.threadId,
      );
    } catch (error) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Le mail n\'a pas pu être marqué lu : $error')),
      );
    }
  }

  void _showRemoteImages() {
    setState(() => _loadRemoteImages = true);
    _load();
  }

  @override
  Widget build(BuildContext context) {
    final thread = _thread;
    return Scaffold(appBar: AppBar(), body: _body(context, thread));
  }

  Widget _body(BuildContext context, ThreadView? thread) {
    final error = _loadError;
    if (error != null) {
      return Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Text(
            'La conversation n\'a pas pu être ouverte : $error',
            textAlign: TextAlign.center,
          ),
        ),
      );
    }
    if (thread == null) {
      return const Center(child: CircularProgressIndicator());
    }
    final theme = Theme.of(context);
    final latest = thread.messages.isEmpty ? null : thread.messages.last.id;
    return ListView(
      key: const Key('thread-view'),
      padding: const EdgeInsets.only(bottom: 32),
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 4, 16, 12),
          child: Text(
            thread.subject.isEmpty ? '(sans objet)' : thread.subject,
            style: theme.textTheme.titleLarge,
          ),
        ),
        if (thread.remoteImagesBlocked)
          _RemoteImagesNotice(onShow: _showRemoteImages),
        for (final message in thread.messages)
          _MessageCard(
            key: ValueKey('${message.id}-$_loadRemoteImages'),
            message: message,
            open: message.id == latest || _opened.contains(message.id),
            onToggle: message.id == latest
                ? null
                : () => setState(() {
                    if (!_opened.remove(message.id)) _opened.add(message.id);
                  }),
          ),
        if (thread.attachments.isNotEmpty)
          _Attachments(attachments: thread.attachments),
      ],
    );
  }
}

class _RemoteImagesNotice extends StatelessWidget {
  const _RemoteImagesNotice({required this.onShow});

  final VoidCallback onShow;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Container(
      margin: const EdgeInsets.fromLTRB(16, 0, 16, 12),
      padding: const EdgeInsets.fromLTRB(12, 4, 4, 4),
      decoration: BoxDecoration(
        color: theme.colorScheme.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(8),
      ),
      child: Row(
        children: [
          const Icon(Icons.image_not_supported_outlined, size: 20),
          const SizedBox(width: 8),
          const Expanded(
            child: Text(
              'Images distantes bloquées pour protéger votre vie privée.',
            ),
          ),
          TextButton(
            key: const Key('show-remote-images'),
            onPressed: onShow,
            child: const Text('Afficher'),
          ),
        ],
      ),
    );
  }
}

class _MessageCard extends StatelessWidget {
  const _MessageCard({
    super.key,
    required this.message,
    required this.open,
    required this.onToggle,
  });

  final MessageView message;
  final bool open;
  final VoidCallback? onToggle;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final initial = message.sender.isEmpty
        ? '?'
        : message.sender.characters.first.toUpperCase();
    final header = InkWell(
      onTap: onToggle,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 16, 8),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            CircleAvatar(radius: 18, child: Text(initial)),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Expanded(
                        child: Text(
                          message.sender.isEmpty
                              ? '(sans expéditeur)'
                              : message.sender,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: theme.textTheme.titleSmall,
                        ),
                      ),
                      Text(
                        _when(message.dateMs.toInt()),
                        style: theme.textTheme.bodySmall,
                      ),
                    ],
                  ),
                  Text(
                    open
                        ? (message.recipients.isEmpty
                              ? ''
                              : 'À : ${message.recipients.join(', ')}')
                        : message.preview,
                    maxLines: open ? 2 : 1,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.bodySmall,
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const Divider(height: 1),
        header,
        if (open) _MailBody(html: message.html),
      ],
    );
  }

  static String _when(int dateMs) {
    final date = DateTime.fromMillisecondsSinceEpoch(dateMs);
    String two(int value) => value.toString().padLeft(2, '0');
    return '${two(date.day)}/${two(date.month)}/${date.year} '
        '${two(date.hour)}:${two(date.minute)}';
  }
}

/// One message body in the system web view, as tall as its content so the
/// conversation scrolls as one page. Scripts in the mail never run; links
/// open outside the app.
class _MailBody extends StatefulWidget {
  const _MailBody({required this.html});

  final String html;

  @override
  State<_MailBody> createState() => _MailBodyState();
}

class _MailBodyState extends State<_MailBody> {
  /// Shrinks a mail laid out wider than the phone to fit it, as Outlook
  /// does, then reports the height the body needs.
  static const _fitAndMeasure = '''
(function () {
  var doc = document.documentElement;
  var body = document.body;
  body.style.zoom = '';
  var wide = doc.scrollWidth;
  var room = window.innerWidth;
  if (wide > room + 1) { body.style.zoom = room / wide; }
  return Math.ceil(doc.scrollHeight * (parseFloat(body.style.zoom) || 1));
})()
''';

  late final WebViewController _controller;
  double _height = 120;

  @override
  void initState() {
    super.initState();
    _controller = WebViewController()
      ..setJavaScriptMode(JavaScriptMode.disabled)
      ..setBackgroundColor(Colors.white)
      ..setNavigationDelegate(
        NavigationDelegate(
          onNavigationRequest: _onNavigation,
          onPageFinished: (_) => _measure(),
        ),
      )
      ..loadHtmlString(widget.html);
  }

  Future<NavigationDecision> _onNavigation(NavigationRequest request) async {
    final uri = Uri.tryParse(request.url);
    if (uri == null || uri.scheme == 'about' || uri.scheme == 'data') {
      return NavigationDecision.navigate;
    }
    await launchUrl(uri, mode: LaunchMode.externalApplication);
    return NavigationDecision.prevent;
  }

  Future<void> _measure() async {
    final result = await _controller.runJavaScriptReturningResult(
      _fitAndMeasure,
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

class _Attachments extends StatelessWidget {
  const _Attachments({required this.attachments});

  final List<AttachmentView> attachments;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const Divider(height: 1),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 16, 4),
          child: Text(
            '${attachments.length} pièce${attachments.length > 1 ? 's' : ''} jointe${attachments.length > 1 ? 's' : ''}',
            style: theme.textTheme.titleSmall,
          ),
        ),
        for (final attachment in attachments)
          ListTile(
            dense: true,
            leading: const Icon(Icons.insert_drive_file_outlined),
            title: Text(
              attachment.filename.isEmpty ? '(sans nom)' : attachment.filename,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
            subtitle: Text(_size(attachment.size?.toInt())),
          ),
      ],
    );
  }

  static String _size(int? bytes) {
    if (bytes == null) return '';
    if (bytes < 1024) return '$bytes o';
    if (bytes < 1024 * 1024) return '${(bytes / 1024).round()} Ko';
    return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} Mo';
  }
}
