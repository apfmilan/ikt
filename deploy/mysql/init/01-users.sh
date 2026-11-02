# Sourced (not executed) by the mysql:8.4 entrypoint on FIRST initialisation only.
# Creates least-privilege users; passwords come from Docker secrets (hex, no quoting issues).
# - tc_app:     DML only (used by api + engine)
# - tc_migrate: DDL on trading_center (used by the one-shot `tc migrate` service)
# NOTE: append-only audit tables are enforced by the migrations (triggers), not here.
tc_app_pw="$(cat /run/secrets/mysql_app_password)"
tc_migrate_pw="$(cat /run/secrets/mysql_migrate_password)"
docker_process_sql --database=mysql <<-SQL
	CREATE USER IF NOT EXISTS 'tc_app'@'%' IDENTIFIED BY '${tc_app_pw}';
	CREATE USER IF NOT EXISTS 'tc_migrate'@'%' IDENTIFIED BY '${tc_migrate_pw}';
	GRANT SELECT, INSERT, UPDATE, DELETE ON \`trading_center\`.* TO 'tc_app'@'%';
	GRANT ALL PRIVILEGES ON \`trading_center\`.* TO 'tc_migrate'@'%';
	FLUSH PRIVILEGES;
SQL
unset tc_app_pw tc_migrate_pw
