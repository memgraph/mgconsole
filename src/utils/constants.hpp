#pragma once

#include <string>
#include <string_view>
#include <vector>

namespace constants {

// Usage strings
constexpr const auto *kUsage =
    "Memgraph bolt client.\n"
    "The client can be run in interactive or non-interactive mode.\n";

constexpr const std::string_view kInteractiveUsage =
    "In interactive mode, user can enter Cypher queries and supported "
    "commands.\n\n"
    "Cypher queries can span through multiple lines and conclude with a\n"
    "semi-colon (;). Each query is executed in the database and the results\n"
    "are printed out.\n\n"
    "The following interactive commands are supported:\n\n"
    "\t:help\t Print out usage for interactive mode\n"
    "\t:quit\t Exit the shell\n";

constexpr const std::string_view kDocs =
    "If you are new to Memgraph or the Cypher query language, check out these "
    "resources:\n\n"
    "\tQuerying with Cypher:  https://memgr.ph/querying\n"
    "\tImporting data:  https://memgr.ph/importing-data\n"
    "\tDatabase configuration:  https://memgr.ph/configuration\n\n"
    "Official mgconsole documentation available on: "
    "https://memgr.ph/mgconsole\n";

constexpr const std::string_view kCommandQuit = ":quit";
constexpr const std::string_view kCommandHelp = ":help";
constexpr const std::string_view kCommandDocs = ":docs";

// Supported formats.
constexpr const std::string_view kCsvFormat = "csv";
constexpr const std::string_view kTabularFormat = "tabular";
constexpr const std::string_view kCypherlFormat = "cypherl";

// Supported modes.
constexpr const std::string_view kSerialMode = "serial";
constexpr const std::string_view kBatchedParallel = "batched-parallel";
constexpr const std::string_view kParserMode = "parser";

// History default directory.
static const std::string kDefaultHistoryBaseDir = "~";
static const std::string kDefaultHistoryMemgraphDir = ".memgraph";
// History filename.
static const std::string kHistoryFilename = "client_history";

static const std::string kPrompt = "memgraph> ";
static const std::string kMultilinePrompt = "       -> ";

/// Memgraph and OpenCypher keywords.
static const std::vector<std::string_view> kMemgraphKeywords{
    "ALTER",      "ASYNC", "AUTH",    "BATCH", "BATCHES", "CLEAR",     "CSV",     "DATA",       "DELIMITER",   "DENY",
    "DROP",       "FOR",   "FREE",    "FROM",  "GRANT",   "HEADER",    "INFO",    "IDENTIFIED", "INTERVAL",    "K_TEST",
    "KAFKA",      "LOAD",  "LOCK",    "MAIN",  "MODE",    "PASSWORD",  "REPLICA", "REPLICAS",   "REPLICATION", "PORT",
    "PRIVILEGES", "QUOTE", "REVOKE",  "ROLE",  "ROLES",   "SIZE",      "START",   "STATS",      "STOP",        "STREAM",
    "STREAMS",    "SYNC",  "TIMEOUT", "TO",    "TOPIC",   "TRANSFORM", "UNLOCK",  "USER",       "USERS"};

static const std::vector<std::string_view> kCypherKeywords{
    "ALL",      "AND",    "ANY",    "AS",         "ASC",    "ASCENDING", "BFS",        "BY",       "CASE",
    "CONTAINS", "COUNT",  "CREATE", "CYPHERNULL", "DELETE", "DESC",      "DESCENDING", "DETACH",   "DISTINCT",
    "ELSE",     "END",    "ENDS",   "EXTRACT",    "FALSE",  "FILTER",    "IN",         "INDEX",    "IS",
    "LIMIT",    "L_SKIP", "MATCH",  "MERGE",      "NONE",   "NOT",       "ON",         "OPTIONAL", "OR",
    "ORDER",    "REDUCE", "REMOVE", "RETURN",     "SET",    "SHOW",      "SINGLE",     "STARTS",   "THEN",
    "TRUE",     "UNION",  "UNWIND", "WHEN",       "WHERE",  "WITH",      "WSHORTEST",  "XOR"};

static const std::vector<std::string_view> kAwesomeFunctions{"DEGREE",
                                                             "INDEGREE",
                                                             "OUTDEGREE",
                                                             "ENDNODE",
                                                             "HEAD",
                                                             "ID",
                                                             "LAST",
                                                             "PROPERTIES",
                                                             "SIZE",
                                                             "STARTNODE",
                                                             "TIMESTAMP",
                                                             "TOBOOLEAN",
                                                             "TOFLOAT",
                                                             "TOINTEGER",
                                                             "TYPE",
                                                             "VALUETYPE",
                                                             "KEYS",
                                                             "LABELS",
                                                             "NODES",
                                                             "RANGE",
                                                             "RELATIONSHIPS",
                                                             "TAIL",
                                                             "UNIFORMSAMPLE",
                                                             "ABS",
                                                             "CEIL",
                                                             "FLOOR",
                                                             "RAND",
                                                             "ROUND",
                                                             "SIGN",
                                                             "E",
                                                             "EXP",
                                                             "LOG",
                                                             "LOG10",
                                                             "SQRT",
                                                             "ACOS",
                                                             "ASIN",
                                                             "ATAN",
                                                             "ATAN2",
                                                             "COS",
                                                             "PI",
                                                             "SIN",
                                                             "TAN",
                                                             "CONTAINS",
                                                             "ENDSWITH",
                                                             "LEFT",
                                                             "LTRIM",
                                                             "REPLACE",
                                                             "REVERSE",
                                                             "RIGHT",
                                                             "RTRIM",
                                                             "SPLIT",
                                                             "STARTSWITH",
                                                             "SUBSTRING",
                                                             "TOLOWER",
                                                             "TOSTRING",
                                                             "TOUPPER",
                                                             "TRIM",
                                                             "ASSERT",
                                                             "COUNTER",
                                                             "TOBYTESTRING",
                                                             "FROMBYTESTRING",
                                                             "DATE",
                                                             "LOCALTIME",
                                                             "LOCALDATETIME",
                                                             "DURATION"};

}  // namespace constants
