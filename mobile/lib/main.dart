import 'package:courly_mobile/src/rust/api/engine.dart';
import 'package:courly_mobile/src/rust/api/microsoft.dart';
import 'package:courly_mobile/src/rust/frb_generated.dart';
import 'package:flutter/services.dart';
import 'package:flutter_web_auth_2/flutter_web_auth_2.dart';
import 'package:material_ui/material_ui.dart';
import 'package:path_provider/path_provider.dart';

Future<void> main() async {
  await RustLib.init();
  runApp(const CourlyApp());
}

/// Starts the Courly engine on the app's private support folder.
Future<EngineReady> startDeviceEngine() async {
  final supportDirectory = await getApplicationSupportDirectory();
  await supportDirectory.create(recursive: true);
  return startEngine(dataDir: supportDirectory.path);
}

/// Runs a Microsoft sign-in in Apple's authentication session and adds
/// the account it returns.
Future<AddedAccount> addMicrosoftAccount() async {
  final signIn = await microsoftSignInStart();
  final callbackUrl = await FlutterWebAuth2.authenticate(
    url: signIn.authUrl,
    callbackUrlScheme: signIn.callbackScheme,
  );
  return microsoftSignInFinish(callbackUrl: callbackUrl);
}

class CourlyApp extends StatelessWidget {
  const CourlyApp({
    super.key,
    this.startEngine = startDeviceEngine,
    this.addAccount = addMicrosoftAccount,
  });

  final Future<EngineReady> Function() startEngine;
  final Future<AddedAccount> Function() addAccount;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Courly',
      home: HomeScreen(startEngine: startEngine, addAccount: addAccount),
    );
  }
}

class HomeScreen extends StatefulWidget {
  const HomeScreen({
    super.key,
    required this.startEngine,
    required this.addAccount,
  });

  final Future<EngineReady> Function() startEngine;
  final Future<AddedAccount> Function() addAccount;

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  late final Future<EngineReady> _engine = widget.startEngine();
  bool _isSigningIn = false;
  AddedAccount? _account;
  String? _signInError;

  Future<void> _signIn() async {
    setState(() {
      _isSigningIn = true;
      _signInError = null;
    });
    try {
      final account = await widget.addAccount();
      setState(() => _account = account);
    } on PlatformException catch (error) {
      // The user closing Apple's sheet is not an error worth showing.
      setState(() {
        _signInError = error.code == 'CANCELED'
            ? null
            : error.message ?? error.code;
      });
    } catch (error) {
      setState(() => _signInError = '$error');
    } finally {
      setState(() => _isSigningIn = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Courly')),
      body: Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: FutureBuilder<EngineReady>(
            future: _engine,
            builder: (context, snapshot) {
              if (snapshot.hasError) {
                return Text(
                  'Le moteur Courly n\'a pas démarré : ${snapshot.error}',
                  key: const Key('engine-error'),
                  textAlign: TextAlign.center,
                );
              }
              if (snapshot.data == null) {
                return const CircularProgressIndicator();
              }
              return _accountSection(context);
            },
          ),
        ),
      ),
    );
  }

  Widget _accountSection(BuildContext context) {
    final account = _account;
    if (account != null) {
      return Text(
        'Compte ajouté\n${account.email}\nSynchronisation en cours',
        key: const Key('account-added'),
        textAlign: TextAlign.center,
        style: Theme.of(context).textTheme.titleMedium,
      );
    }
    return Column(
      mainAxisSize: MainAxisSize.min,
      key: const Key('engine-ready'),
      children: [
        FilledButton(
          key: const Key('add-microsoft'),
          onPressed: _isSigningIn ? null : _signIn,
          child: Text(
            _isSigningIn ? 'Connexion…' : 'Ajouter un compte Microsoft',
          ),
        ),
        if (_signInError != null) ...[
          const SizedBox(height: 16),
          Text(
            _signInError!,
            key: const Key('sign-in-error'),
            textAlign: TextAlign.center,
            style: TextStyle(color: Theme.of(context).colorScheme.error),
          ),
        ],
      ],
    );
  }
}
