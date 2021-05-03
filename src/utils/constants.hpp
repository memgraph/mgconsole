#pragma once

#include <string>
#include <vector>

namespace constants {

// Usage strings
constexpr const auto *kUsage =
    "Memgraph bolt client.\n"
    "The client can be run in interactive or non-interactive mode.\n";

constexpr const auto *kInteractiveUsage =
    "In interactive mode, user can enter cypher queries and supported "
    "commands.\n\n"
    "Cypher queries can span through multiple lines and conclude with a\n"
    "semi-colon (;). Each query is executed in the database and the results\n"
    "are printed out.\n\n"
    "The following interactive commands are supported:\n\n"
    "\t:help\t Print out usage for interactive mode\n"
    "\t:quit\t Exit the shell\n";

static const std::string kCommandQuit = ":quit";
static const std::string kCommandHelp = ":help";

// Supported formats.
static const std::string kCsvFormat = "csv";
static const std::string kTabularFormat = "tabular";

// History default directory.
static const std::string kDefaultHistoryBaseDir = "~";
static const std::string kDefaultHistoryMemgraphDir = ".memgraph";
// History filename.
static const std::string kHistoryFilename = "client_history";

static const std::string kPrompt = "memgraph> ";
static const std::string kMultilinePrompt = "       -> ";

/// Memgraph and OpenCypher keywords.
static const std::vector<std::string> kMemgraphKeywords{
    "ALTER",      "ASYNC",   "AUTH",       "BATCH",       "BATCHES",
    "CLEAR",      "CSV",     "DATA",       "DELIMITER",   "DENY",
    "DROP",       "FOR",     "FREE",       "FROM",        "GRANT",
    "HEADER",     "INFO",    "IDENTIFIED", "INTERVAL",    "K_TEST",
    "KAFKA",      "LOAD",    "LOCK",       "MAIN",        "MODE",
    "PASSWORD",   "REPLICA", "REPLICAS",   "REPLICATION", "PORT",
    "PRIVILEGES", "QUOTE",   "REVOKE",     "ROLE",        "ROLES",
    "SIZE",       "START",   "STATS",      "STOP",        "STREAM",
    "STREAMS",    "SYNC",    "TIMEOUT",    "TO",          "TOPIC",
    "TRANSFORM",  "UNLOCK",  "USER",       "USERS"};

static const std::vector<std::string> kCypherKeywords{
    "ALL",        "AND",    "ANY",    "AS",         "ASC",      "ASCENDING",
    "BFS",        "BY",     "CASE",   "CONTAINS",   "COUNT",    "CREATE",
    "CYPHERNULL", "DELETE", "DESC",   "DESCENDING", "DETACH",   "DISTINCT",
    "ELSE",       "END",    "ENDS",   "EXTRACT",    "FALSE",    "FILTER",
    "IN",         "INDEX",  "IS",     "LIMIT",      "L_SKIP",   "MATCH",
    "MERGE",      "NONE",   "NOT",    "ON",         "OPTIONAL", "OR",
    "ORDER",      "REDUCE", "REMOVE", "RETURN",     "SET",      "SHOW",
    "SINGLE",     "STARTS", "THEN",   "TRUE",       "UNION",    "UNWIND",
    "WHEN",       "WHERE",  "WITH",   "WSHORTEST",  "XOR"};

}  // namespace constants
