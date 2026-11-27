import 'package:courly_mobile/src/rust/api/mailbox.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:material_ui/material_ui.dart';

/// How the phone names and draws each system folder.
extension MailFolderLook on MailFolder {
  String get label => switch (this) {
    MailFolder.inbox => 'Boîte de réception',
    MailFolder.sent => 'Envoyés',
    MailFolder.archive => 'Archives',
    MailFolder.trash => 'Corbeille',
    MailFolder.spam => 'Courrier indésirable',
  };

  IconData get icon => switch (this) {
    MailFolder.inbox => LucideIcons.inbox,
    MailFolder.sent => LucideIcons.send,
    MailFolder.archive => LucideIcons.archive,
    MailFolder.trash => LucideIcons.trash2,
    MailFolder.spam => LucideIcons.shieldAlert,
  };

  /// Whether conversations can be archived or deleted from this folder.
  /// The engine files a conversation out of the inbox only: elsewhere the
  /// row would stay in the list after the action.
  bool get canFile => this == MailFolder.inbox;
}
