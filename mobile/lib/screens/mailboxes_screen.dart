import 'package:courly_mobile/mail_folders.dart';
import 'package:courly_mobile/screens/add_account_screen.dart';
import 'package:courly_mobile/screens/inbox_screen.dart';
import 'package:courly_mobile/src/rust/api/mailbox.dart';
import 'package:courly_mobile/widgets/account_avatar.dart';
import 'package:courly_mobile/widgets/glass.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:material_ui/material_ui.dart';

/// The mailboxes page, as in Apple Mail: every inbox together and each
/// account's own, then one section per account with its other folders,
/// each row with its unread count, and "Add an account" below.
/// The app opens straight on the inbox of every account, pushed over this
/// page, so its back button leads here.
class MailboxesScreen extends StatefulWidget {
  const MailboxesScreen({super.key, required this.dataDir});

  final String dataDir;

  @override
  State<MailboxesScreen> createState() => _MailboxesScreenState();
}

class _MailboxesScreenState extends State<MailboxesScreen> {
  /// The folders listed under each account, the inbox being above.
  static const _accountFolders = [
    MailFolder.sent,
    MailFolder.archive,
    MailFolder.trash,
    MailFolder.spam,
  ];

  List<AccountSummary> _accounts = const [];
  List<FolderUnread> _unread = const [];
  String? _loadError;

  @override
  void initState() {
    super.initState();
    _reload();
    WidgetsBinding.instance.addPostFrameCallback(
      (_) => _openInbox(null, animate: false),
    );
  }

  Future<void> _reload() async {
    try {
      final accounts = await listAccounts(dataDir: widget.dataDir);
      final unread = await unreadCounts(dataDir: widget.dataDir);
      if (!mounted) return;
      setState(() {
        _accounts = accounts;
        _unread = unread;
        _loadError = null;
      });
    } catch (error) {
      if (!mounted) return;
      setState(() => _loadError = '$error');
    }
  }

  /// The unread conversations of [folder] in [accountId], or in every
  /// account's inbox when it is null.
  int _unreadIn(String? accountId, MailFolder folder) {
    for (final count in _unread) {
      if (count.accountId == accountId && count.folder == folder) {
        return count.unread;
      }
    }
    return 0;
  }

  /// Every account's inbox when [accountId] is null. The first one, at
  /// launch, appears without sliding in.
  Future<void> _openInbox(
    String? accountId, {
    MailFolder folder = MailFolder.inbox,
    bool animate = true,
  }) async {
    final page = InboxScreen(
      dataDir: widget.dataDir,
      accountId: accountId,
      folder: folder,
    );
    await Navigator.of(context).push(
      animate
          ? MaterialPageRoute<void>(builder: (_) => page)
          : PageRouteBuilder<void>(
              pageBuilder: (_, _, _) => page,
              transitionDuration: Duration.zero,
              reverseTransitionDuration: const Duration(milliseconds: 300),
            ),
    );
    _reload();
  }

  Future<void> _addAccount() async {
    final added = await showAddAccountSheet(context);
    if (added == true) _reload();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final error = _loadError;
    return GlassPage(
      backgroundColor: scheme.surfaceContainer,
      body: ListView(
        key: const Key('mailboxes'),
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 8, 20, 16),
            child: Text(
              'Boîtes aux lettres',
              style: theme.textTheme.headlineMedium,
            ),
          ),
          if (error != null)
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 20),
              child: Text(
                'Les comptes n\'ont pas pu être lus : $error',
                style: TextStyle(color: scheme.error),
              ),
            ),
          if (_accounts.isNotEmpty)
            _Section(
              children: [
                _MailboxRow(
                  key: const Key('mailbox-all'),
                  leading: _RoundIcon(
                    icon: LucideIcons.inbox,
                    color: scheme.primary,
                  ),
                  title: 'Toutes les boîtes de réception',
                  subtitle:
                      '${_accounts.length} compte${_accounts.length > 1 ? 's' : ''}',
                  unread: _unreadIn(null, MailFolder.inbox),
                  onTap: () => _openInbox(null),
                ),
                for (final account in _accounts)
                  _MailboxRow(
                    key: ValueKey('mailbox-${account.id}'),
                    leading: AccountAvatar(account: account, size: 32),
                    title: account.name,
                    subtitle: account.email,
                    unread: _unreadIn(account.id, MailFolder.inbox),
                    onTap: () => _openInbox(account.id),
                  ),
              ],
            ),
          for (final account in _accounts) ...[
            Padding(
              padding: const EdgeInsets.fromLTRB(32, 0, 32, 8),
              child: Text(
                account.name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: theme.textTheme.titleSmall?.copyWith(
                  color: scheme.onSurfaceVariant,
                ),
              ),
            ),
            _Section(
              children: [
                for (final folder in _accountFolders)
                  _MailboxRow(
                    key: ValueKey('folder-${account.id}-${folder.name}'),
                    leading: Icon(folder.icon, size: 20, color: scheme.primary),
                    title: folder.label,
                    unread: _unreadIn(account.id, folder),
                    onTap: () => _openInbox(account.id, folder: folder),
                  ),
              ],
            ),
          ],
          _Section(
            children: [
              _MailboxRow(
                key: const Key('drawer-add-account'),
                leading: _RoundIcon(
                  icon: LucideIcons.plus,
                  color: scheme.onSurfaceVariant,
                ),
                title: 'Ajouter un compte',
                onTap: _addAccount,
                showChevron: false,
              ),
            ],
          ),
        ],
      ),
    );
  }
}

/// A grouped, rounded section of rows, as in iOS Settings and Mail.
class _Section extends StatelessWidget {
  const _Section({required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final rows = <Widget>[];
    for (var index = 0; index < children.length; index++) {
      if (index > 0) {
        rows.add(
          Padding(
            padding: const EdgeInsets.only(left: 64),
            child: Divider(height: 0.6, color: scheme.outlineVariant),
          ),
        );
      }
      rows.add(children[index]);
    }
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 0, 16, 24),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(16),
        child: Material(
          color: scheme.surface,
          child: Column(children: rows),
        ),
      ),
    );
  }
}

class _MailboxRow extends StatelessWidget {
  const _MailboxRow({
    super.key,
    required this.leading,
    required this.title,
    required this.onTap,
    this.subtitle,
    this.unread = 0,
    this.showChevron = true,
  });

  final Widget leading;
  final String title;
  final String? subtitle;

  /// The unread count shown before the chevron, when there is one.
  final int unread;
  final VoidCallback onTap;
  final bool showChevron;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final detail = subtitle;
    return InkWell(
      onTap: onTap,
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 56),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 10, 12, 10),
          child: Row(
            children: [
              SizedBox(width: 36, child: Center(child: leading)),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(
                      title,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.titleMedium,
                    ),
                    if (detail != null)
                      Text(
                        detail,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: theme.textTheme.bodySmall,
                      ),
                  ],
                ),
              ),
              if (unread > 0)
                Padding(
                  padding: const EdgeInsets.only(left: 8, right: 4),
                  child: Text(
                    '$unread',
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: scheme.onSurfaceVariant,
                    ),
                  ),
                ),
              if (showChevron)
                Icon(
                  LucideIcons.chevronRight,
                  size: 18,
                  color: scheme.onSurfaceVariant,
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class _RoundIcon extends StatelessWidget {
  const _RoundIcon({required this.icon, required this.color});

  final IconData icon;
  final Color color;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: 32,
      height: 32,
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.12),
        shape: BoxShape.circle,
      ),
      child: Icon(icon, size: 17, color: color),
    );
  }
}
