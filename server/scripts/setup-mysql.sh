#!/usr/bin/env bash
# Create the portfolio database and its two accounts, and write their connection strings to .env.
#   portfolio_app    the website: reads content, adds contact messages, nothing else
#   portfolio_admin  `portfolio seed`: creates tables and replaces content
# Passwords are random and go straight into .env; they are never printed.
# Usage: scripts/setup-mysql.sh [mysql client args for an admin login, e.g. -u root -p]
set -euo pipefail
cd "$(dirname "$0")/.."

MYSQL=${MYSQL:-mysql}
DB=${PORTFOLIO_DB:-portfolio}
HOST=${PORTFOLIO_DB_HOST:-127.0.0.1}
APP_PW=$(openssl rand -hex 24)
ADMIN_PW=$(openssl rand -hex 24)

"$MYSQL" -h "$HOST" "$@" <<SQL
CREATE DATABASE IF NOT EXISTS \`$DB\` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
CREATE USER IF NOT EXISTS 'portfolio_app'@'localhost' IDENTIFIED BY '$APP_PW';
CREATE USER IF NOT EXISTS 'portfolio_app'@'127.0.0.1' IDENTIFIED BY '$APP_PW';
ALTER USER 'portfolio_app'@'localhost' IDENTIFIED BY '$APP_PW';
ALTER USER 'portfolio_app'@'127.0.0.1' IDENTIFIED BY '$APP_PW';
CREATE USER IF NOT EXISTS 'portfolio_admin'@'localhost' IDENTIFIED BY '$ADMIN_PW';
CREATE USER IF NOT EXISTS 'portfolio_admin'@'127.0.0.1' IDENTIFIED BY '$ADMIN_PW';
ALTER USER 'portfolio_admin'@'localhost' IDENTIFIED BY '$ADMIN_PW';
ALTER USER 'portfolio_admin'@'127.0.0.1' IDENTIFIED BY '$ADMIN_PW';
GRANT ALL PRIVILEGES ON \`$DB\`.* TO 'portfolio_admin'@'localhost', 'portfolio_admin'@'127.0.0.1';
GRANT SELECT ON \`$DB\`.* TO 'portfolio_app'@'localhost', 'portfolio_app'@'127.0.0.1';
SQL

# messages may not exist yet: create it as admin first so the INSERT grant has a table
"$MYSQL" -h "$HOST" "$@" "$DB" <<SQL
CREATE TABLE IF NOT EXISTS messages (id BIGINT AUTO_INCREMENT PRIMARY KEY, \`name\` VARCHAR(100) NOT NULL, email VARCHAR(200) NOT NULL, body TEXT NOT NULL, received DATETIME NOT NULL, KEY (received)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
GRANT INSERT ON \`$DB\`.messages TO 'portfolio_app'@'localhost', 'portfolio_app'@'127.0.0.1';
SQL

touch .env
chmod 600 .env 2>/dev/null || true
grep -v -E '^(MYSQL_DSN|MYSQL_ADMIN_DSN)=' .env > .env.tmp || true
{
  echo "MYSQL_DSN=mysql://portfolio_app:$APP_PW@$HOST:3306/$DB"
  echo "MYSQL_ADMIN_DSN=mysql://portfolio_admin:$ADMIN_PW@$HOST:3306/$DB"
} >> .env.tmp
mv .env.tmp .env
echo "MySQL ready: database $DB, accounts portfolio_app (read + insert messages) and portfolio_admin; connection strings written to .env"
