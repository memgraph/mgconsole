[![Actions Status](https://github.com/memgraph/mgconsole/workflows/CI/badge.svg)](https://github.com/memgraph/mgconsole/actions)

# mgconsole

mgconsole is a command line interface for [Memgraph](https://memgraph.com)
database.

<img width="630" alt="mgconsole" src="https://github.com/memgraph/mgconsole/assets/4950251/b7ce1a0d-097c-4a2f-81b5-4049a307668b">

## Building and installing

To build and install mgconsole from source you will need:
  - CMake version >= 3.4
  - OpenSSL version >= 1.0.2
  - C compiler supporting C11
  - C++ compiler supporting C++20

To install compile dependencies on Debian / Ubuntu:

```
apt-get install -y git cmake make gcc g++ libssl-dev
```

On RedHat / CentOS / Fedora:

```
yum install -y git cmake make gcc gcc-c++ openssl-devel libstdc++-static
```

On MacOS, first make sure you have [XCode](https://developer.apple.com/xcode/) and [Homebrew](https://brew.sh) installed. Then, in the terminal, paste:

```
brew install git cmake make openssl
```

On Windows, you need to install the MSYS2. Just follow the [instructions](https://www.msys2.org), up to step 6.
In addition, OpenSSL must be installed. You can easily install it with an
[installer](https://slproweb.com/products/Win32OpenSSL.html). The Win64
version is required, although the "Light" version is enough. Both EXE and MSI
variants should work.
Then, you'll need to install the dependencies using the MSYS2 MINGW64 terminal,
which should be available from your Start menu. Just run the following command
inside the MSYS2 MINGW64 terminal:

```
pacman -Syu --needed base-devel git mingw-w64-x86_64-toolchain mingw-w64-x86_64-cmake mingw-w64-x86_64-openssl
```

Once everything is in place, create a build directory inside the source
directory and configure the build by running CMake from it as follows:

* on Linux:
```
mkdir build
cd build
cmake -DCMAKE_BUILD_TYPE=Release ..
```

* on MacOS:
```
mkdir build
cd build
cmake -DOPENSSL_ROOT_DIR="$(brew --prefix openssl)" -DCMAKE_BUILD_TYPE=Release ..
```

* on Windows, from the MSYS2 MINGW64 terminal:
```
mkdir build
cd build
cmake -G "MinGW Makefiles" -DCMAKE_BUILD_TYPE=Release ..
```

After running CMake, you should see a Makefile in the build directory. Then you
can build the project by running:
```
make
```

This will build the `mgconsole` binary. To install it, run:
```
make install
```

This will install to system default installation directory. If you want to
change this location, use `-DCMAKE_INSTALL_PREFIX` option when running CMake.

NOTE: If you have issues compiling `mgconsole` using your compiler, please try to use
[Memgraph official toolchain](https://memgraph.notion.site/Toolchain-37c37c84382149a58d09b2ccfcb410d7).
In case you encounter any problem, please create
[a new GitHub issue](https://github.com/memgraph/mgconsole/issues/new).

## Example usage

```
$ mgconsole --host 127.0.0.1 --port 7687 --use-ssl=false
mgconsole 0.1
Type :help for shell usage
Quit the shell by typing Ctrl-D(eof) or :quit
Connected to 'memgraph://127.0.0.1:7687'
memgraph> :help
In interactive mode, user can enter cypher queries and supported commands.

Cypher queries can span through multiple lines and conclude with a
semi-colon (;). Each query is executed in the database and the results
are printed out.

The following interactive commands are supported:

        :help    Print out usage for interactive mode
        :quit    Exit the shell

memgraph>
memgraph> MATCH (t:Turtle) RETURN t;
+-------------------------------------------+
| t                                         |
+-------------------------------------------+
| (:Turtle {color: "blue", name: "Leo"})    |
| (:Turtle {color: "purple", name: "Don"})  |
| (:Turtle {color: "orange", name: "Mike"}) |
| (:Turtle {color: "red", name: "Raph"})    |
+-------------------------------------------+
4 rows in set (0.000 sec)
memgraph> :quit
Bye
```

## Export & import into Memgraph

An interesting use-case for `mgconsole` is exporting and importing data.
You can close the loop by running the following example queries:

```
# Export to cypherl formatted data file
echo "DUMP DATABASE;" | mgconsole --output-format=cypherl > data.cypherl

# Import from cypherl file
cat data.cypherl | mgconsole
```

## Batched and parallelized import (EXPERIMENTAL)

Since Memgraph v2 expects vertices to come first (vertices has to exist to
create an edge), and serial import can be slow, the goal with batching and
parallelization is to improve the import speed when ingesting queries in the
text format.

To enable faster import, use `--import-mode="batched-parallel"` flag when
running `mgconsole` + put Memgraph into the `STORAGE MODE
IN_MEMORY_ANALYTICAL;` (could be part of the `.cypherl` file) to be able to
leverage parallelism in the best possible way.

```
cat data.cypherl | mgconsole --import-mode=batched-parallel
# STORAGE MODE IN_MEMORY_ANALYTICAL; is optional
```

IMPORTANT NOTE: Inside the import file, vertices always have to come first
because `mgconsole` will read the file serially and chunk by chunk.

Additional useful runtime flags are:
  - `--batch-size=10000`
  - `--workers-number=64`

### Memgraph in the TRANSACTIONAL mode

In [TRANSACTIONAL
mode](https://memgraph.com/docs/memgraph/reference-guide/storage-modes#transactional-storage-mode-default),
batching and parallelization might help, but since there are high chances for
serialization errors, the execution times might be similar or even slower
compared to the serial mode.

### Memgraph in ANALYTICAL mode

In [ANALYTICAL
mode](https://memgraph.com/docs/memgraph/reference-guide/storage-modes#analytical-storage-mode),
batching and parallelization will mostly likely help massively because
serialization errors don't exist, but since Memgraph will accept any query
(e.g., on edge create failure, vertices could be created multiple times),
special care is required:
  - queries with pure create vertices have to be specified first
  - please use only import statements using simple MATCH, CREATE, MERGE
    statements.

If you encounter any issue, please create a new [mgconsole GitHub issue](https://github.com/memgraph/mgconsole/issues).
