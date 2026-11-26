import 'dart:async';

import 'package:courly_mobile/screens/add_account_screen.dart';
import 'package:courly_mobile/screens/compose_screen.dart';
import 'package:courly_mobile/src/rust/api/actions.dart';
import 'package:courly_mobile/src/rust/api/compose.dart';
import 'package:courly_mobile/screens/thread_screen.dart';
import 'package:courly_mobile/src/rust/api/mailbox.dart';
import 'package:courly_mobile/theme.dart';
import 'package:courly_mobile/widgets/glass.dart';
import 'package:courly_mobile/widgets/sender_avatar.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:material_ui/material_ui.dart';

/// The inbox of every account, or of [accountId] alone, opened over the
/// mailboxes page as in Apple Mail: the glass back button returns to it.
class InboxScreen extends StatefulWidget {
  const InboxScreen({super.key, required this.dataDir, this.accountId});

  final String dataDir;

  /// The account shown, or every account when null.
  final String? accountId;

  @override
  State<InboxScreen> createState() => _InboxScreenState();
}

class _InboxScreenState extends State<InboxScreen> {
  /// How often the list is read again while accounts sync.
  static const _refreshEvery = Duration(seconds: 5);

  List<AccountSummary> _accounts = const [];
  List<ThreadSummary> _threads = const [];
  String? _loadError;
  bool _loaded = false;
  Timer? _refresh;

  @override
  void initState() {
    super.initState();
    _reload();
    _refresh = Timer.periodic(_refreshEvery, (_) => _reload());
  }

  @override
  void dispose() {
    _refresh?.cancel();
    super.dispose();
  }

  Future<void> _reload() async {
    try {
      final accounts = await listAccounts(dataDir: widget.dataDir);
      final threads = await inboxThreads(
        dataDir: widget.dataDir,
        accountId: widget.accountId,
        limit: 100,
      );
      if (!mounted) return;
      setState(() {
        _accounts = accounts;
        _threads = threads;
        _loadError = null;
        _loaded = true;
      });
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _loadError = '$error';
        _loaded = true;
      });
    }
  }

  Future<void> _addAccount() async {
    final added = await showAddAccountSheet(context);
    if (added == true) _reload();
  }

  Future<void> _openThread(ThreadSummary thread) async {
    final outcome = await Navigator.of(context).push<ThreadOutcome>(
      MaterialPageRoute(
        builder: (_) => ThreadScreen(
          dataDir: widget.dataDir,
          accountId: thread.accountId,
          threadId: thread.id,
          markRead: !thread.isRead,
        ),
      ),
    );
    if (outcome != null) {
      _tell(
        outcome == ThreadOutcome.archived
            ? 'Conversation archivée'
            : 'Conversation supprimée',
      );
    }
    _reload();
  }

  /// Archive or delete straight from the list: the row leaves at once and
  /// comes back if the engine refuses.
  Future<void> _actOnRow(ThreadSummary thread, ThreadOutcome outcome) async {
    setState(
      () => _threads = [
        for (final other in _threads)
          if (other.id != thread.id || other.accountId != thread.accountId)
            other,
      ],
    );
    try {
      final run = outcome == ThreadOutcome.archived
          ? archiveThread
          : trashThread;
      await run(accountId: thread.accountId, threadId: thread.id);
      _tell(
        outcome == ThreadOutcome.archived
            ? 'Conversation archivée'
            : 'Conversation supprimée',
      );
    } catch (error) {
      _tell('L\'action a échoué : $error');
    }
    _reload();
  }

  Future<void> _newMessage() async {
    final accountId = widget.accountId ?? _accounts.first.id;
    final sent = await Navigator.of(context).push<bool>(
      MaterialPageRoute(
        fullscreenDialog: true,
        builder: (_) => ComposeScreen(
          dataDir: widget.dataDir,
          mode: ComposeMode.new_,
          accountId: accountId,
        ),
      ),
    );
    if (sent == true) _tell('Message envoyé');
  }

  void _tell(String text) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));
  }

  AccountSummary? get _selectedAccount {
    for (final account in _accounts) {
      if (account.id == widget.accountId) return account;
    }
    return null;
  }

  @override
  Widget build(BuildContext context) {
    final selected = _selectedAccount;
    final theme = Theme.of(context);
    return GlassPage(
      leading: GlassIconButton(
        key: const Key('open-menu'),
        symbol: 'chevron.left',
        icon: LucideIcons.chevronLeft,
        label: 'Boîtes aux lettres',
        onPressed: () => Navigator.of(context).maybePop(),
      ),
      actions: [
        if (_accounts.isNotEmpty)
          AdaptiveAppBarAction(
            iosSymbol: 'square.and.pencil',
            icon: LucideIcons.squarePen,
            label: 'Nouveau message',
            onPressed: _newMessage,
          ),
      ],
      body: RefreshIndicator(
        onRefresh: _reload,
        child: CustomScrollView(
          key: const Key('inbox-list'),
          physics: const AlwaysScrollableScrollPhysics(),
          slivers: [
            SliverSafeArea(
              bottom: false,
              sliver: SliverToBoxAdapter(
                child: Padding(
                  padding: const EdgeInsets.fromLTRB(20, 8, 20, 8),
                  child: Text(
                    selected?.name ?? 'Boîte de réception',
                    style: theme.textTheme.headlineMedium,
                  ),
                ),
              ),
            ),
            ..._body(context),
          ],
        ),
      ),
    );
  }

  List<Widget> _body(BuildContext context) {
    Widget fill(Widget child) =>
        SliverFillRemaining(hasScrollBody: false, child: child);
    if (!_loaded) {
      return [fill(const Center(child: CircularProgressIndicator()))];
    }
    final error = _loadError;
    if (error != null) {
      return [
        fill(
          Center(
            child: Padding(
              padding: const EdgeInsets.all(24),
              child: Text(
                'La boîte de réception n\'a pas pu être lue : $error',
                textAlign: TextAlign.center,
              ),
            ),
          ),
        ),
      ];
    }
    if (_accounts.isEmpty) {
      return [fill(_NoAccount(onAddAccount: _addAccount))];
    }
    if (_threads.isEmpty) {
      return [
        fill(
          Center(
            child: Text(
              'Synchronisation en cours…',
              style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                color: Theme.of(context).colorScheme.onSurfaceVariant,
              ),
            ),
          ),
        ),
      ];
    }
    return [
      SliverPadding(
        padding: const EdgeInsets.only(bottom: 96),
        sliver: SliverList.builder(
          itemCount: _threads.length,
          itemBuilder: (context, index) {
            final thread = _threads[index];
            return Dismissible(
              key: ValueKey('thread-${thread.accountId}-${thread.id}'),
              background: const _SwipeAction(
                color: CourlyTheme.archive,
                icon: LucideIcons.archive,
                label: 'Archiver',
                alignment: Alignment.centerLeft,
              ),
              secondaryBackground: const _SwipeAction(
                color: CourlyTheme.delete,
                icon: LucideIcons.trash2,
                label: 'Supprimer',
                alignment: Alignment.centerRight,
              ),
              onDismissed: (direction) => _actOnRow(
                thread,
                direction == DismissDirection.startToEnd
                    ? ThreadOutcome.archived
                    : ThreadOutcome.deleted,
              ),
              child: _ThreadRow(
                thread: thread,
                onTap: () => _openThread(thread),
              ),
            );
          },
        ),
      ),
    ];
  }
}

/// What shows under a row while it is swiped, as in Outlook: archive to
/// the right in green, delete to the left in red.
class _SwipeAction extends StatelessWidget {
  const _SwipeAction({
    required this.color,
    required this.icon,
    required this.label,
    required this.alignment,
  });

  final Color color;
  final IconData icon;
  final String label;
  final Alignment alignment;

  @override
  Widget build(BuildContext context) {
    return ColoredBox(
      color: color,
      child: Align(
        alignment: alignment,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 28),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(icon, color: Colors.white, size: 22),
              const SizedBox(height: 6),
              Text(
                label,
                style: const TextStyle(
                  color: Colors.white,
                  fontSize: 12.5,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _NoAccount extends StatelessWidget {
  const _NoAccount({required this.onAddAccount});

  final VoidCallback onAddAccount;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Center(
      key: const Key('no-account'),
      child: Padding(
        padding: const EdgeInsets.all(32),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: 72,
              height: 72,
              decoration: BoxDecoration(
                color: theme.colorScheme.primaryContainer,
                shape: BoxShape.circle,
              ),
              child: Icon(
                LucideIcons.mail,
                size: 32,
                color: theme.colorScheme.primary,
              ),
            ),
            const SizedBox(height: 20),
            Text(
              'Aucun compte',
              style: theme.textTheme.titleLarge?.copyWith(
                fontWeight: FontWeight.w700,
              ),
            ),
            const SizedBox(height: 8),
            Text(
              'Ajoutez votre adresse Outlook, Gmail, Exchange, iCloud ou autre.',
              textAlign: TextAlign.center,
              style: theme.textTheme.bodyMedium?.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
              ),
            ),
            const SizedBox(height: 28),
            FilledButton(
              key: const Key('add-account'),
              onPressed: onAddAccount,
              style: FilledButton.styleFrom(
                minimumSize: const Size(220, 50),
                shape: RoundedRectangleBorder(
                  borderRadius: BorderRadius.circular(14),
                ),
                textStyle: const TextStyle(
                  fontFamily: 'Inter',
                  fontSize: 16,
                  fontWeight: FontWeight.w600,
                ),
              ),
              child: const Text('Ajouter un compte'),
            ),
          ],
        ),
      ),
    );
  }
}

/// One conversation in the list, compact as in Outlook: the sender's
/// coloured initials, their name and the time on one line, the subject,
/// then one line of preview. Unread ones carry a teal dot and bold text.
class _ThreadRow extends StatelessWidget {
  const _ThreadRow({required this.thread, required this.onTap});

  final ThreadSummary thread;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final unread = !thread.isRead;
    final sender = thread.sender.isEmpty ? '(sans expéditeur)' : thread.sender;
    return Material(
      color: scheme.surface,
      child: InkWell(
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(0, 12, 16, 0),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              SizedBox(
                width: 16,
                height: 40,
                child: unread
                    ? Center(
                        child: Container(
                          key: const Key('unread-dot'),
                          width: 8,
                          height: 8,
                          decoration: BoxDecoration(
                            color: scheme.primary,
                            shape: BoxShape.circle,
                          ),
                        ),
                      )
                    : null,
              ),
              SenderAvatar(name: sender),
              const SizedBox(width: 12),
              Expanded(
                child: DecoratedBox(
                  decoration: BoxDecoration(
                    border: Border(
                      bottom: BorderSide(
                        color:
                            theme.dividerTheme.color ?? scheme.outlineVariant,
                        width: theme.dividerTheme.thickness ?? 0.6,
                      ),
                    ),
                  ),
                  child: Padding(
                    padding: const EdgeInsets.only(bottom: 12),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Row(
                          children: [
                            Expanded(
                              child: Text(
                                sender,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: theme.textTheme.titleMedium?.copyWith(
                                  fontWeight: unread
                                      ? FontWeight.w700
                                      : FontWeight.w500,
                                ),
                              ),
                            ),
                            if (thread.hasAttachments)
                              Padding(
                                padding: const EdgeInsets.only(right: 6),
                                child: Icon(
                                  LucideIcons.paperclip,
                                  size: 14,
                                  color: scheme.onSurfaceVariant,
                                ),
                              ),
                            Text(
                              _when(thread.dateMs),
                              style: theme.textTheme.bodySmall?.copyWith(
                                color: unread
                                    ? scheme.primary
                                    : scheme.onSurfaceVariant,
                                fontWeight: unread
                                    ? FontWeight.w600
                                    : FontWeight.w400,
                              ),
                            ),
                          ],
                        ),
                        const SizedBox(height: 2),
                        Text(
                          thread.subject.isEmpty
                              ? '(sans objet)'
                              : thread.subject,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: theme.textTheme.bodyMedium?.copyWith(
                            fontWeight: unread
                                ? FontWeight.w600
                                : FontWeight.w400,
                          ),
                        ),
                        const SizedBox(height: 2),
                        Text(
                          thread.snippet,
                          maxLines: 2,
                          overflow: TextOverflow.ellipsis,
                          style: theme.textTheme.bodyMedium?.copyWith(
                            color: scheme.onSurfaceVariant,
                            fontSize: 13.5,
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  static const _weekdays = [
    'lun.',
    'mar.',
    'mer.',
    'jeu.',
    'ven.',
    'sam.',
    'dim.',
  ];

  /// The time today, "Hier", the weekday within the week, else the date,
  /// as Outlook's list shows it.
  static String _when(int? dateMs) {
    if (dateMs == null) return '';
    final date = DateTime.fromMillisecondsSinceEpoch(dateMs);
    final now = DateTime.now();
    final today = DateTime(now.year, now.month, now.day);
    final day = DateTime(date.year, date.month, date.day);
    final daysAgo = today.difference(day).inDays;
    String two(int value) => value.toString().padLeft(2, '0');
    if (daysAgo <= 0) return '${two(date.hour)}:${two(date.minute)}';
    if (daysAgo == 1) return 'Hier';
    if (daysAgo < 7) return _weekdays[date.weekday - 1];
    if (date.year == now.year) return '${two(date.day)}/${two(date.month)}';
    return '${two(date.day)}/${two(date.month)}/${date.year % 100}';
  }
}
