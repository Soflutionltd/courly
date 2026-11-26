import 'package:adaptive_platform_ui/adaptive_platform_ui.dart'
    show
        AdaptiveAppBar,
        AdaptiveAppBarAction,
        AdaptiveScaffold,
        IOS26Button,
        IOS26ButtonSize,
        IOS26ButtonStyle,
        PlatformInfo,
        SFSymbol;
import 'package:material_ui/material_ui.dart';

export 'package:adaptive_platform_ui/adaptive_platform_ui.dart'
    show AdaptiveAppBarAction;

/// A screen under iOS 26's own Liquid Glass bar: UIKit's toolbar holds the
/// [leading] button, the [title] and the [actions] as native glass, and the
/// [body] scrolls underneath it, fading out under the status bar.
///
/// The app's own [Scaffold] stays the outer one, so the drawer, snack bars
/// and dialogs of `material_ui` keep working; the glass toolbar comes from
/// `adaptive_platform_ui`, which falls back to Cupertino bars before iOS 26
/// and Material ones on Android.
class GlassPage extends StatelessWidget {
  const GlassPage({
    super.key,
    required this.body,
    this.title,
    this.leading,
    this.actions = const [],
    this.bottom,
    this.drawer,
    this.scaffoldKey,
  });

  /// The page content. It sees the bar's height in its top padding, so a
  /// [SafeArea], a [SliverSafeArea] or a [ListView] without its own padding
  /// starts below the bar and scrolls under it.
  final Widget body;
  final String? title;
  final Widget? leading;
  final List<AdaptiveAppBarAction> actions;

  /// Floats over the bottom of the page, above the home indicator.
  final Widget? bottom;
  final Widget? drawer;
  final GlobalKey<ScaffoldState>? scaffoldKey;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final floating = bottom;
    return Scaffold(
      key: scaffoldKey,
      drawer: drawer,
      body: AdaptiveScaffold(
        backgroundColor: scheme.surface,
        appBar: AdaptiveAppBar(
          useNativeToolbar: true,
          title: title,
          tintColor: scheme.primary,
          leading: leading,
          actions: actions,
        ),
        body: Stack(
          children: [
            Positioned.fill(child: body),
            const ScrollEdgeFade(),
            if (floating != null) const ScrollEdgeFade(atBottom: true),
            if (floating != null)
              Positioned(
                left: 0,
                right: 0,
                bottom: 0,
                child: SafeArea(
                  top: false,
                  child: Padding(
                    padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
                    child: floating,
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// iOS 26's scroll edge effect, which UIKit only draws over its own scroll
/// views: content fades out under the status bar and the glass bar instead
/// of showing through at full contrast.
class ScrollEdgeFade extends StatelessWidget {
  const ScrollEdgeFade({super.key, this.atBottom = false});

  /// Under floating bottom buttons instead of under the top bar.
  final bool atBottom;

  @override
  Widget build(BuildContext context) {
    final surface = Theme.of(context).colorScheme.surface;
    final padding = MediaQuery.paddingOf(context);
    return Positioned(
      top: atBottom ? null : 0,
      bottom: atBottom ? 0 : null,
      left: 0,
      right: 0,
      height: atBottom ? padding.bottom + 72 : padding.top + 12,
      child: IgnorePointer(
        child: DecoratedBox(
          decoration: BoxDecoration(
            gradient: LinearGradient(
              begin: atBottom ? Alignment.bottomCenter : Alignment.topCenter,
              end: atBottom ? Alignment.topCenter : Alignment.bottomCenter,
              colors: [
                surface,
                surface.withValues(alpha: 0.85),
                surface.withValues(alpha: 0),
              ],
              stops: const [0, 0.55, 1],
            ),
          ),
        ),
      ),
    );
  }
}

/// A round Liquid Glass button with an SF Symbol on iOS 26, an icon button
/// with the matching Lucide [icon] elsewhere.
class GlassIconButton extends StatelessWidget {
  const GlassIconButton({
    super.key,
    required this.symbol,
    required this.icon,
    required this.label,
    required this.onPressed,
  });

  final String symbol;
  final IconData icon;
  final String label;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    if (!PlatformInfo.isIOS26OrHigher()) {
      return IconButton(
        tooltip: label,
        onPressed: onPressed,
        icon: Icon(icon, size: 22),
      );
    }
    return Semantics(
      label: label,
      button: true,
      child: IOS26Button.sfSymbol(
        onPressed: onPressed,
        sfSymbol: SFSymbol(
          symbol,
          size: 17,
          color: Theme.of(context).colorScheme.onSurface,
        ),
        size: IOS26ButtonSize.large,
        minSize: const Size(44, 44),
        useSmoothRectangleBorder: false,
      ),
    );
  }
}

/// A round Liquid Glass button around any [child] on iOS 26, such as an
/// account badge; a plain icon button elsewhere.
class GlassChildButton extends StatelessWidget {
  const GlassChildButton({
    super.key,
    required this.label,
    required this.onPressed,
    required this.child,
  });

  final String label;
  final VoidCallback onPressed;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    if (!PlatformInfo.isIOS26OrHigher()) {
      return IconButton(tooltip: label, onPressed: onPressed, icon: child);
    }
    return Semantics(
      label: label,
      button: true,
      child: IOS26Button.child(
        onPressed: onPressed,
        style: IOS26ButtonStyle.glass,
        size: IOS26ButtonSize.large,
        minSize: const Size(44, 44),
        useSmoothRectangleBorder: false,
        child: child,
      ),
    );
  }
}

/// A labelled Liquid Glass capsule on iOS 26, an outlined button elsewhere.
class GlassLabelButton extends StatelessWidget {
  const GlassLabelButton({
    super.key,
    required this.label,
    required this.icon,
    required this.onPressed,
  });

  final String label;
  final IconData icon;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    if (!PlatformInfo.isIOS26OrHigher()) {
      return OutlinedButton.icon(
        onPressed: onPressed,
        icon: Icon(icon, size: 18),
        label: Text(label),
      );
    }
    return IOS26Button(
      onPressed: onPressed,
      label: label,
      style: IOS26ButtonStyle.glass,
      size: IOS26ButtonSize.large,
      textColor: Theme.of(context).colorScheme.onSurface,
      useSmoothRectangleBorder: false,
    );
  }
}
