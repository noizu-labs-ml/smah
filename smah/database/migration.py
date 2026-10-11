import argparse
import hashlib
import os
import sys
import time
import importlib
import textwrap
import sqlite3
from smah.database.database import Database
from smah.exit_codes import ExitCode

class Migration:
    # Migrations are stored in the `migrations` directory
    MIGRATIONS_DIR = os.path.join(os.path.dirname(__file__), "migrations")

    def __init__(self):
        pass


    @staticmethod
    def get_schema_migrations(database: Database):
        cursor = database.connection.cursor()
        create_table = textwrap.dedent(
            """
            CREATE TABLE IF NOT EXISTS schema_migrations(
                migration VARCHAR(255) PRIMARY KEY, 
                checksum CHAR(32), 
                applied BOOLEAN DEFAULT FALSE, 
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, 
                modified_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
            """
        ).strip()
        cursor.execute(create_table)

        cursor.execute("SELECT migration, checksum, applied, created_at, modified_at FROM schema_migrations ORDER BY migration ASC")
        result = cursor.fetchall()
        cursor.close()
        migrations = []
        for row in result:
            migration, checksum, applied, created_at, modified_at = row
            migrations.append(
                {
                    'migration': migration,
                    'checksum': checksum,
                    'applied': applied == 1,
                    'created_at': created_at,
                    'modified_at': modified_at
                }
            )
        return migrations

    @staticmethod
    def get_migrations():
        migrations = []
        os.makedirs(Migration.MIGRATIONS_DIR, exist_ok=True)
        for migration in os.listdir(Migration.MIGRATIONS_DIR):
            if migration.endswith(".py"):
                digest = hashlib.md5(open(os.path.join(Migration.MIGRATIONS_DIR, migration), "rb").read()).hexdigest()
                migrations.append({'file': migration, 'checksum': digest})
        return migrations

    @staticmethod
    def apply_migration(database: Database, migration: dict, silent: bool = False, exit_on_finish: bool = True) -> tuple:
        cursor = database.connection.cursor()
        module = importlib.import_module(f"smah.database.migrations.{migration['file'][:-3]}")
        cursor.execute("BEGIN TRANSACTION")
        try:
            module.up(cursor)
            cursor.execute(
            """
                INSERT INTO schema_migrations (migration, checksum, applied) 
                VALUES (?, ?, ?)
                ON CONFLICT(migration) DO UPDATE SET
                    checksum = excluded.checksum,
                    applied = excluded.applied
                """,
                (migration['file'], migration['checksum'], True)
            )
            cursor.execute("COMMIT")
            cursor.close()
            r = f"Applied Migration {migration['file']}"
            if not silent:
                print(r)
            return "success", r

        except Exception as e:
            cursor.execute("ROLLBACK")
            cursor.close()
            r = f"Error Applying Migration {migration['file']}: {e}"
            if not silent:
                print(r)
            if exit_on_finish:
                sys.exit(ExitCode.UNEXPECTED_ERROR)
            return ("error", ("Migration Failed", r))




    @staticmethod
    def rollback_migration(database: Database, migration: dict, silent: bool = False, exit_on_finish: bool = True) -> tuple:
        cursor = database.connection.cursor()
        module = importlib.import_module(f"smah.database.migrations.{migration['file'][:-3]}")
        cursor.execute("BEGIN TRANSACTION")
        try:
            module.down(cursor)
            cursor.execute(
                """
                INSERT INTO schema_migrations (migration, checksum, applied) 
                VALUES (?, ?, ?)
                ON CONFLICT(migration) DO UPDATE SET
                    checksum = excluded.checksum,
                    applied = excluded.applied
                """,
                (migration['file'], migration['checksum'], False)
            )
            cursor.execute("COMMIT")
            cursor.close()
            r = f"Reverted Migration '{migration['file']}'"
            if not silent:
                print(r)
            return "success", r
        except Exception as e:
            cursor.execute("ROLLBACK")
            cursor.close()
            r = f"Error Reverting Migration '{migration['file']}': {e}"
            if not silent:
                print(r)
            if exit_on_finish:
                sys.exit(ExitCode.UNEXPECTED_ERROR)
            return "error", ("Rollback Failed", r)

    @staticmethod
    def migrate(
            database: Database,
            args: argparse.Namespace,
            silent: bool = False,
            exit_on_finish: bool = True
    ) -> tuple:
        count = 0
        available_migrations = Migration.get_migrations()
        tracked_migrations = {m['migration']: m for m in Migration.get_schema_migrations(database)}

        for migration in available_migrations:
            # Skip migrations that have already been applied
            tracked = tracked_migrations.get(migration['file'])
            if tracked and tracked['applied']:
                if tracked['checksum'] != migration['checksum']:

                    r = f"Checksum Mismatch in {migration['file']}: expected {tracked['checksum']} but got {migration['checksum']}"
                    if not silent:
                        print(r)

                    if args.reset_checksums:
                        cursor = database.connection.cursor()
                        cursor.execute("UPDATE schema_migrations SET checksum = ? WHERE migration = ?", (migration['checksum'], migration['file']))
                        cursor.close()
                    else:
                        if exit_on_finish:
                            sys.exit(ExitCode.UNEXPECTED_ERROR)
                        else:
                            return "error", ("Checksum Mismatch", r)

                if args.to and args.to == migration['file']:
                    r = f"Migration Complete: changes applied {count}"
                    if not silent:
                        print(r)
                    if exit_on_finish:
                        sys.exit(ExitCode.OK)
                    else:
                        return "success", r
            else:
                outcome, details = Migration.apply_migration(database, migration, silent=silent, exit_on_finish=exit_on_finish)
                if outcome == "error":
                    if exit_on_finish:
                        sys.exit(ExitCode.UNEXPECTED_ERROR)
                    else:
                        return outcome, details

                count += 1
                if args.count and count >= args.count:
                    r = f"Migration Complete: changes applied {count}"
                    if not silent:
                        print(r)
                    if exit_on_finish:
                        sys.exit(ExitCode.OK)
                    else:
                        return "success", r

                if args.to and args.to == migration['file']:
                    r = f"Migration Complete: changes applied {count}"
                    if not silent:
                        print(r)

                    if exit_on_finish:
                        sys.exit(ExitCode.OK)
                    else:
                        return "success", r

        if count == 0:
            r = "No Migrations Pending"
            if not silent:
                print(r)
            return "nop", r
        else:
            r = f"Migration Complete: changes applied {count}"
            if not silent:
                print(r)
            return "success", r


    @staticmethod
    def rollback(database: Database, args: argparse.Namespace):
        count = 0
        available_migrations = {m['file']: m for m in Migration.get_migrations()}
        tracked_migrations = [m for m in Migration.get_schema_migrations(database) if m['applied'] and m['migration'] in available_migrations]
        tracked_migrations.reverse()
        if args.to:
            # Verify to target is applied
            available = False
            for migration in tracked_migrations:
                if migration['migration'] == args.to:
                    available = True
                    break
            if not available:
                print(f"Migration --to '{args.to}' not applied or available")
                sys.exit(ExitCode.USAGE)
        for migration in tracked_migrations:
            Migration.rollback_migration(database, available_migrations[migration['migration']])
            count += 1
            if args.count and count >= args.count:
                print(f"Rollback Complete: changes applied {count}")
                sys.exit(ExitCode.OK)
            if args.to and args.to == migration['migration']:
                print(f"Rollback Complete: changes applied {count}")
                sys.exit(ExitCode.OK)
        if count == 0:
            print("No Migrations Available for Rollback")
        else:
            print(f"Rollback Complete #{count} migrations reverted.")

    @staticmethod
    def status(database: Database):
        available_migrations = Migration.get_migrations()
        tracked_migrations = {m['migration']: m for m in Migration.get_schema_migrations(database)}
        if not available_migrations:
            print("No Migrations Available")
            sys.exit(ExitCode.OK)
        out = ""
        for migration in available_migrations:
            migration['applied'] = False
            migration['checksum_mismatch'] = False
            tracked = tracked_migrations.get(migration['file'])
            if tracked:
                migration['applied'] = tracked['applied']
                if tracked['checksum'] != migration['checksum']:
                    migration['checksum_mismatch'] = True
            out += f"{migration['file']} {'(applied)' if migration['applied'] else ''} {'(checksum mismatch)' if migration['checksum_mismatch'] else ''}\n"
        print(f"Migrations:\n{out}")

    @staticmethod
    def create(name: str) -> None:
        epoch = int(time.time())
        template = textwrap.dedent(
            """
            def up(cursor):
                \"\"\"
                Apply schema.
                \"\"\"
                pass
                
            def down(cursor):
                \"\"\"
                Rollback schema.
                \"\"\"
                pass    
         
            """).strip()
        mf = os.path.join(Migration.MIGRATIONS_DIR, f"{epoch}_{name}.py")
        os.makedirs(Migration.MIGRATIONS_DIR, exist_ok=True)
        with open(mf, "w") as file:
            file.write(template)
        print(f"Created migration: {mf}")

