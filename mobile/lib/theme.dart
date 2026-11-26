import 'package:material_ui/material_ui.dart';

/// Courly's look on the phone: the desktop palette (Outlook's neutrals and
/// the teal of the logo), Inter, and flat white or dark surfaces with none
/// of Material's tinted backgrounds.
abstract final class CourlyTheme {
  /// The logo's teal, darkened until white text on it reads at 4.8 to 1,
  /// as on the desktop.
  static const _tealLight = Color(0xFF0C7F7D);

  /// The logo's teal as it is, which needs no darkening on a dark ground.
  static const _tealDark = Color(0xFF16B3B0);

  /// Archive green and delete red of the swipe actions, the desktop's
  /// success and danger colours.
  static const archive = Color(0xFF107C10);
  static const delete = Color(0xFFC50F1F);

  static ThemeData light() => _theme(
    const ColorScheme(
      brightness: Brightness.light,
      primary: _tealLight,
      onPrimary: Colors.white,
      primaryContainer: Color(0xFFE3F2F2),
      onPrimaryContainer: Color(0xFF064241),
      secondary: _tealLight,
      onSecondary: Colors.white,
      error: delete,
      onError: Colors.white,
      surface: Colors.white,
      onSurface: Color(0xFF242424),
      onSurfaceVariant: Color(0xFF6B6B6B),
      surfaceContainerLowest: Colors.white,
      surfaceContainerLow: Color(0xFFFAFAFA),
      surfaceContainer: Color(0xFFF5F5F5),
      surfaceContainerHigh: Color(0xFFF0F0F0),
      surfaceContainerHighest: Color(0xFFEBEBEB),
      outline: Color(0xFFD1D1D1),
      outlineVariant: Color(0xFFEDEDED),
      surfaceTint: Colors.transparent,
    ),
  );

  static ThemeData dark() => _theme(
    const ColorScheme(
      brightness: Brightness.dark,
      primary: _tealDark,
      onPrimary: Color(0xFF00201F),
      primaryContainer: Color(0xFF0E3D3C),
      onPrimaryContainer: Color(0xFFBDEBEA),
      secondary: _tealDark,
      onSecondary: Color(0xFF00201F),
      error: Color(0xFFDC626D),
      onError: Colors.black,
      surface: Color(0xFF1B1B1B),
      onSurface: Color(0xFFF5F5F5),
      onSurfaceVariant: Color(0xFFA8A8A8),
      surfaceContainerLowest: Color(0xFF141414),
      surfaceContainerLow: Color(0xFF202020),
      surfaceContainer: Color(0xFF262626),
      surfaceContainerHigh: Color(0xFF2E2E2E),
      surfaceContainerHighest: Color(0xFF363636),
      outline: Color(0xFF4A4A4A),
      outlineVariant: Color(0xFF2E2E2E),
      surfaceTint: Colors.transparent,
    ),
  );

  static ThemeData _theme(ColorScheme scheme) {
    final base = ThemeData(
      colorScheme: scheme,
      fontFamily: 'Inter',
      scaffoldBackgroundColor: scheme.surface,
      splashFactory: InkSparkle.splashFactory,
    );
    final text = base.textTheme.apply(
      bodyColor: scheme.onSurface,
      displayColor: scheme.onSurface,
    );
    return base.copyWith(
      textTheme: text.copyWith(
        headlineMedium: text.headlineMedium?.copyWith(
          fontSize: 30,
          fontWeight: FontWeight.w700,
          letterSpacing: -0.6,
        ),
        titleMedium: text.titleMedium?.copyWith(
          fontSize: 16,
          letterSpacing: -0.2,
        ),
        bodyMedium: text.bodyMedium?.copyWith(
          fontSize: 14.5,
          letterSpacing: -0.1,
          height: 1.3,
        ),
        bodySmall: text.bodySmall?.copyWith(
          fontSize: 13,
          color: scheme.onSurfaceVariant,
        ),
      ),
      appBarTheme: AppBarTheme(
        backgroundColor: scheme.surface,
        foregroundColor: scheme.onSurface,
        surfaceTintColor: Colors.transparent,
        scrolledUnderElevation: 0,
        elevation: 0,
        centerTitle: false,
      ),
      dividerTheme: DividerThemeData(
        color: scheme.outlineVariant,
        thickness: 0.6,
        space: 0.6,
      ),
      floatingActionButtonTheme: FloatingActionButtonThemeData(
        backgroundColor: scheme.primary,
        foregroundColor: scheme.onPrimary,
        elevation: 3,
        highlightElevation: 5,
        shape: const CircleBorder(),
      ),
      snackBarTheme: SnackBarThemeData(
        behavior: SnackBarBehavior.floating,
        backgroundColor: scheme.onSurface,
        contentTextStyle: TextStyle(
          fontFamily: 'Inter',
          color: scheme.surface,
          fontSize: 14,
        ),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
      ),
      drawerTheme: DrawerThemeData(
        backgroundColor: scheme.surface,
        surfaceTintColor: Colors.transparent,
      ),
      progressIndicatorTheme: ProgressIndicatorThemeData(color: scheme.primary),
    );
  }
}
