[![Actions Status](https://github.com/memgraph/mgconsole/workflows/build-test/badge.svg)](https://github.com/memgraph/mgconsole/actions)

# mgconsole

mgconsole is a command line interface for [Memgraph](https://memgraph.com)
database.

## Building and installing

To build and install mgconsole from source you will need:
  - CMake version >= 3.4
  - OpenSSL version >= 1.0.2
  - C compiler supporting C11
  - C++ compiler supporting C++14 with support for Filesystem TS (at least
    experimental)
  - [mgclient](https://github.com/memgraph/mgclient) library and headers
  - [readline](https://tiswww.case.edu/php/chet/readline/rltop.html) library
    and headers

To install compile dependencies on Debian / Ubuntu:

```
apt-get install -y git cmake make gcc g++ libssl-dev libreadline-dev
```

On RedHat / CentOS / Fedora:

```
yum install -y git cmake make gcc gcc-c++ openssl-devel readline-devel
```

Once everything is in place (please make sure `mgclient` is installed), create
a build directory inside the source directory and configure the build by
running CMake from it:

```
mkdir build
cd build
cmake ..
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
