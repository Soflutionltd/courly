import 'package:courly_mobile/src/rust/api/mailbox.dart';
import 'package:material_ui/material_ui.dart';

/// An account's round badge: the first letter of its name on its colour.
class AccountAvatar extends StatelessWidget {
  const AccountAvatar({super.key, required this.account, required this.size});

  final AccountSummary account;
  final double size;

  @override
  Widget build(BuildContext context) {
    final background =
        _parseHex(account.color) ?? Theme.of(context).colorScheme.primary;
    final letter = account.name.isEmpty
        ? '?'
        : account.name.characters.first.toUpperCase();
    return CircleAvatar(
      radius: size / 2,
      backgroundColor: background,
      child: Text(
        letter,
        style: TextStyle(
          color: Colors.white,
          fontSize: size * 0.42,
          fontWeight: FontWeight.w500,
        ),
      ),
    );
  }

  /// `#rrggbb` as a colour, or `null` for anything else.
  static Color? _parseHex(String? hex) {
    if (hex == null) return null;
    final digits = hex.startsWith('#') ? hex.substring(1) : hex;
    if (digits.length != 6) return null;
    final value = int.tryParse(digits, radix: 16);
    return value == null ? null : Color(0xFF000000 | value);
  }
}
