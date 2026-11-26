import 'package:material_ui/material_ui.dart';

/// A sender's round badge: their initials on a colour picked from their
/// name, the same colour every time, so a sender is recognised at a glance.
class SenderAvatar extends StatelessWidget {
  const SenderAvatar({super.key, required this.name, this.size = 40});

  final String name;
  final double size;

  /// Muted tones that keep white initials readable, in light and dark.
  static const _palette = [
    Color(0xFF0C7F7D),
    Color(0xFF3B5BA9),
    Color(0xFF8A4FB2),
    Color(0xFFB5487A),
    Color(0xFFC0573A),
    Color(0xFFA77B12),
    Color(0xFF4E7D33),
    Color(0xFF55667A),
  ];

  /// Up to two letters: the first of the first and last words.
  static String initials(String name) {
    final words = name
        .replaceAll(RegExp(r'[<>"@].*$'), '')
        .split(RegExp(r'\s+'))
        .where((word) => word.isNotEmpty)
        .toList();
    if (words.isEmpty) {
      return name.isEmpty ? '?' : name.characters.first.toUpperCase();
    }
    final first = words.first.characters.first;
    final last = words.length > 1 ? words.last.characters.first : '';
    return '$first$last'.toUpperCase();
  }

  @override
  Widget build(BuildContext context) {
    final hash = name.codeUnits.fold<int>(
      0,
      (value, unit) => (value * 31 + unit) & 0x7fffffff,
    );
    return Container(
      width: size,
      height: size,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: _palette[hash % _palette.length],
        shape: BoxShape.circle,
      ),
      child: Text(
        initials(name),
        style: TextStyle(
          color: Colors.white,
          fontSize: size * 0.38,
          fontWeight: FontWeight.w600,
          letterSpacing: 0.2,
        ),
      ),
    );
  }
}
