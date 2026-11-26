import 'package:courly_mobile/screens/inbox_screen.dart';
import 'package:courly_mobile/src/rust/api/engine.dart';
import 'package:courly_mobile/src/rust/frb_generated.dart';
import 'package:material_ui/material_ui.dart';
import 'package:path_provider/path_provider.dart';

Future<void> main() async {
  await RustLib.init();
  runApp(const CourlyApp());
}

/// The app's private support folder, where the Courly engine keeps its
/// database, its key and its stores.
Future<String> deviceDataDir() async {
  final supportDirectory = await getApplicationSupportDirectory();
  await supportDirectory.create(recursive: true);
  return supportDirectory.path;
}

/// Starts the Courly engine on the device data folder and returns it.
Future<String> startDeviceEngine() async {
  final dataDir = await deviceDataDir();
  await startEngine(dataDir: dataDir);
  return dataDir;
}

class CourlyApp extends StatelessWidget {
  const CourlyApp({super.key, this.startEngine = startDeviceEngine});

  final Future<String> Function() startEngine;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Courly',
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(seedColor: const Color(0xFF16B3B0)),
      ),
      home: _EngineGate(startEngine: startEngine),
    );
  }
}

/// Shows the inbox once the engine has booted, or why it could not.
class _EngineGate extends StatefulWidget {
  const _EngineGate({required this.startEngine});

  final Future<String> Function() startEngine;

  @override
  State<_EngineGate> createState() => _EngineGateState();
}

class _EngineGateState extends State<_EngineGate> {
  late final Future<String> _dataDir = widget.startEngine();

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<String>(
      future: _dataDir,
      builder: (context, snapshot) {
        if (snapshot.hasError) {
          return Scaffold(
            body: Center(
              child: Padding(
                padding: const EdgeInsets.all(24),
                child: Text(
                  'Le moteur Courly n\'a pas démarré : ${snapshot.error}',
                  key: const Key('engine-error'),
                  textAlign: TextAlign.center,
                ),
              ),
            ),
          );
        }
        final dataDir = snapshot.data;
        if (dataDir == null) {
          return const Scaffold(
            body: Center(child: CircularProgressIndicator()),
          );
        }
        return InboxScreen(dataDir: dataDir);
      },
    );
  }
}
