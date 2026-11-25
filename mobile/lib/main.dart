import 'package:courly_mobile/src/rust/api/database.dart';
import 'package:courly_mobile/src/rust/frb_generated.dart';
import 'package:material_ui/material_ui.dart';
import 'package:path_provider/path_provider.dart';

Future<void> main() async {
  await RustLib.init();
  runApp(const CourlyApp());
}

/// Opens the Courly database in the app's private support folder.
Future<DatabaseReport> openDeviceDatabase() async {
  final supportDirectory = await getApplicationSupportDirectory();
  await supportDirectory.create(recursive: true);
  return openCourlyDatabase(dataDir: supportDirectory.path);
}

class CourlyApp extends StatelessWidget {
  const CourlyApp({super.key, this.loadReport = openDeviceDatabase});

  final Future<DatabaseReport> Function() loadReport;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Courly',
      home: Scaffold(
        appBar: AppBar(title: const Text('Courly')),
        body: Center(
          child: FutureBuilder<DatabaseReport>(
            future: loadReport(),
            builder: (context, snapshot) {
              if (snapshot.hasError) {
                return Text(
                  'Database error: ${snapshot.error}',
                  key: const Key('database-error'),
                  textAlign: TextAlign.center,
                );
              }
              final report = snapshot.data;
              if (report == null) {
                return const CircularProgressIndicator();
              }
              return Text(
                'Courly core ready\n'
                'Schema version ${report.schemaVersion}\n'
                '${report.tableCount} tables\n'
                '${report.migrationsApplied} migrations applied',
                key: const Key('database-report'),
                textAlign: TextAlign.center,
              );
            },
          ),
        ),
      ),
    );
  }
}
