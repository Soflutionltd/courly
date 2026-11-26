import 'dart:async';

import 'package:courly_mobile/screens/add_account_screen.dart';
import 'package:courly_mobile/screens/thread_screen.dart';
import 'package:courly_mobile/src/rust/api/mailbox.dart';
import 'package:courly_mobile/widgets/account_avatar.dart';
import 'package:material_ui/material_ui.dart';

/// The inbox of every account, or of the one chosen in the side menu, as
/// in Outlook: the account column and the folders slide in from the left,
/// and the "+" at the bottom of the column adds an account.
class InboxScreen extends StatefulWidget {
  const InboxScreen({super.key, required this.dataDir});

  final String dataDir;

  @override
  State<InboxScreen> createState() => _InboxScreenState();
}

class _InboxScreenState extends State<InboxScreen> {
  /// How often the list is read again while accounts sync.
  static const _refreshEvery = Duration(seconds: 5);

  List<AccountSummary> _accounts = const [];
  List<ThreadSummary> _threads = const [];
  String? _accountId;
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
        accountId: _accountId,
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

  void _showAccount(String? accountId) {
    setState(() => _accountId = accountId);
    Navigator.of(context).pop();
    _reload();
  }

  Future<void> _addAccount() async {
    final added = await showAddAccountSheet(context);
    if (added == true) _reload();
  }

  Future<void> _openThread(ThreadSummary thread) async {
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => ThreadScreen(
          dataDir: widget.dataDir,
          accountId: thread.accountId,
          threadId: thread.id,
          markRead: !thread.isRead,
        ),
      ),
    );
    _reload();
  }

  AccountSummary? get _selectedAccount {
    for (final account in _accounts) {
      if (account.id == _accountId) return account;
    }
    return null;
  }

  @override
  Widget build(BuildContext context) {
    final selected = _selectedAccount;
    return Scaffold(
      appBar: AppBar(
        title: Text(selected?.name ?? 'Boîte de réception'),
        leading: Builder(
          builder: (context) => IconButton(
            key: const Key('open-menu'),
            tooltip: 'Comptes et dossiers',
            icon: selected == null
                ? const Icon(Icons.menu)
                : AccountAvatar(account: selected, size: 32),
            onPressed: () => Scaffold.of(context).openDrawer(),
          ),
        ),
      ),
      drawer: _AccountDrawer(
        accounts: _accounts,
        selectedId: _accountId,
        onShowAccount: _showAccount,
        onAddAccount: () {
          // Synchronous, so the drawer is gone before the sheet is pushed;
          // `maybePop` would resolve later and close the sheet instead.
          Navigator.of(context).pop();
          _addAccount();
        },
      ),
      body: _body(context),
    );
  }

  Widget _body(BuildContext context) {
    if (!_loaded) {
      return const Center(child: CircularProgressIndicator());
    }
    final error = _loadError;
    if (error != null) {
      return Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Text(
            'La boîte de réception n\'a pas pu être lue : $error',
            textAlign: TextAlign.center,
          ),
        ),
      );
    }
    if (_accounts.isEmpty) {
      return _NoAccount(onAddAccount: _addAccount);
    }
    if (_threads.isEmpty) {
      return RefreshIndicator(
        onRefresh: _reload,
        child: ListView(
          children: const [
            SizedBox(height: 160),
            Center(child: Text('Synchronisation en cours…')),
          ],
        ),
      );
    }
    return RefreshIndicator(
      onRefresh: _reload,
      child: ListView.separated(
        key: const Key('inbox-list'),
        itemCount: _threads.length,
        separatorBuilder: (_, _) => const Divider(height: 1),
        itemBuilder: (context, index) {
          final thread = _threads[index];
          return _ThreadRow(
            key: ValueKey('thread-${thread.id}'),
            thread: thread,
            onTap: () => _openThread(thread),
          );
        },
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
            Icon(
              Icons.mail_outline,
              size: 56,
              color: theme.colorScheme.primary,
            ),
            const SizedBox(height: 16),
            Text('Aucun compte', style: theme.textTheme.titleLarge),
            const SizedBox(height: 8),
            const Text(
              'Ajoutez votre adresse Outlook, Gmail, Exchange, iCloud ou autre.',
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: 24),
            FilledButton(
              key: const Key('add-account'),
              onPressed: onAddAccount,
              child: const Text('Ajouter un compte'),
            ),
          ],
        ),
      ),
    );
  }
}

class _ThreadRow extends StatelessWidget {
  const _ThreadRow({super.key, required this.thread, required this.onTap});

  final ThreadSummary thread;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final unread = !thread.isRead;
    final weight = unread ? FontWeight.w700 : FontWeight.w400;
    return ListTile(
      onTap: onTap,
      title: Row(
        children: [
          Expanded(
            child: Text(
              thread.sender.isEmpty ? '(sans expéditeur)' : thread.sender,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(fontWeight: weight),
            ),
          ),
          Text(
            _when(thread.dateMs),
            style: theme.textTheme.bodySmall?.copyWith(
              color: unread ? theme.colorScheme.primary : null,
            ),
          ),
        ],
      ),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            thread.subject.isEmpty ? '(sans objet)' : thread.subject,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontWeight: weight,
              color: theme.colorScheme.onSurface,
            ),
          ),
          Text(thread.snippet, maxLines: 2, overflow: TextOverflow.ellipsis),
        ],
      ),
      trailing: thread.hasAttachments
          ? const Icon(Icons.attach_file, size: 18)
          : null,
    );
  }

  /// Today's time, else the day and month, as Outlook's list shows it.
  static String _when(int? dateMs) {
    if (dateMs == null) return '';
    final date = DateTime.fromMillisecondsSinceEpoch(dateMs);
    final now = DateTime.now();
    final sameDay =
        date.year == now.year && date.month == now.month && date.day == now.day;
    String two(int value) => value.toString().padLeft(2, '0');
    return sameDay
        ? '${two(date.hour)}:${two(date.minute)}'
        : '${two(date.day)}/${two(date.month)}';
  }
}

/// The side menu: the account column on the left, as in Outlook, with
/// "all accounts" on top and "+" at the bottom, and the folders of the
/// chosen account on the right.
class _AccountDrawer extends StatelessWidget {
  const _AccountDrawer({
    required this.accounts,
    required this.selectedId,
    required this.onShowAccount,
    required this.onAddAccount,
  });

  final List<AccountSummary> accounts;
  final String? selectedId;
  final ValueChanged<String?> onShowAccount;
  final VoidCallback onAddAccount;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    AccountSummary? selected;
    for (final account in accounts) {
      if (account.id == selectedId) selected = account;
    }
    return Drawer(
      child: SafeArea(
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              width: 76,
              child: ListView(
                padding: const EdgeInsets.symmetric(vertical: 12),
                children: [
                  _ColumnButton(
                    selected: selectedId == null,
                    tooltip: 'Tous les comptes',
                    onTap: () => onShowAccount(null),
                    child: const CircleAvatar(
                      radius: 24,
                      child: Icon(Icons.public),
                    ),
                  ),
                  for (final account in accounts)
                    _ColumnButton(
                      selected: account.id == selectedId,
                      tooltip: account.email,
                      onTap: () => onShowAccount(account.id),
                      child: AccountAvatar(account: account, size: 48),
                    ),
                  _ColumnButton(
                    key: const Key('drawer-add-account'),
                    selected: false,
                    tooltip: 'Ajouter un compte',
                    onTap: onAddAccount,
                    child: CircleAvatar(
                      radius: 24,
                      backgroundColor:
                          theme.colorScheme.surfaceContainerHighest,
                      child: const Icon(Icons.add),
                    ),
                  ),
                ],
              ),
            ),
            const VerticalDivider(width: 1),
            Expanded(
              child: ListView(
                padding: const EdgeInsets.symmetric(vertical: 12),
                children: [
                  ListTile(
                    title: Text(
                      selected?.name ?? 'Tous les comptes',
                      style: theme.textTheme.titleMedium,
                    ),
                    subtitle: Text(
                      selected?.email ??
                          '${accounts.length} compte${accounts.length > 1 ? 's' : ''}',
                    ),
                  ),
                  ListTile(
                    leading: Icon(
                      Icons.inbox,
                      color: theme.colorScheme.primary,
                    ),
                    title: Text(
                      'Boîte de réception',
                      style: TextStyle(color: theme.colorScheme.primary),
                    ),
                    selected: true,
                    onTap: () => Navigator.of(context).maybePop(),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _ColumnButton extends StatelessWidget {
  const _ColumnButton({
    super.key,
    required this.selected,
    required this.tooltip,
    required this.onTap,
    required this.child,
  });

  final bool selected;
  final String tooltip;
  final VoidCallback onTap;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final color = Theme.of(context).colorScheme.primary;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Tooltip(
        message: tooltip,
        child: InkResponse(
          onTap: onTap,
          radius: 30,
          child: Container(
            padding: const EdgeInsets.all(2),
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              border: Border.all(
                color: selected ? color : Colors.transparent,
                width: 2,
              ),
            ),
            child: child,
          ),
        ),
      ),
    );
  }
}
