[![Actions Status](https://github.com/memgraph/mgconsole/workflows/CI/badge.svg)](https://github.com/memgraph/mgconsole/actions)

# mgconsole

mgconsole is a command line interface for [Memgraph](https://memgraph.com)
database.

## Building and installing

To build and install mgconsole from source you will need:
  - CMake version >= 3.4
  - OpenSSL version >= 1.0.2
  - C compiler supporting C11
  - C++ compiler supporting C++17

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
