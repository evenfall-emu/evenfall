# Builds tools/bench/harness/vbam-harness.cpp against the VBA-M core that
# Delta ships, with the source set and compiler settings of GBADeltaCore's
# libVBA-M target:
#   cmake -S tools/bench/harness/vbam -B BUILD \
#     -DVBAM_SOURCE=refs/vbam -DGBADELTACORE_SOURCE=refs/gbadeltacore
#   cmake --build BUILD --target vbam-harness
#
# The Release configuration of that Xcode target sets no optimization level,
# so it builds at Xcode's default, -Os. VBAM_OPT overrides it for comparison.
cmake_minimum_required(VERSION 3.24)
project(evenfall-bench-vbam LANGUAGES C CXX)

if(NOT VBAM_SOURCE OR NOT GBADELTACORE_SOURCE)
  message(FATAL_ERROR "set VBAM_SOURCE and GBADELTACORE_SOURCE to their checkouts")
endif()
set(VBAM_OPT "-Os" CACHE STRING "optimization flag for VBA-M and the harness")
# CMake's per-configuration flags would add their own -O level.
set(CMAKE_BUILD_TYPE "" CACHE STRING "" FORCE)

find_package(ZLIB REQUIRED)

set(V ${VBAM_SOURCE})
set(SFML ${GBADELTACORE_SOURCE}/SFML)
add_executable(vbam-harness
  ${CMAKE_CURRENT_LIST_DIR}/../vbam-harness.cpp
  # VBA-M: the GBA and GB cores, sound, common code and archive readers.
  ${V}/src/Util.cpp
  ${V}/src/apu/Blip_Buffer.cpp
  ${V}/src/apu/Effects_Buffer.cpp
  ${V}/src/apu/Gb_Apu.cpp
  ${V}/src/apu/Gb_Apu_State.cpp
  ${V}/src/apu/Gb_Oscs.cpp
  ${V}/src/apu/Multi_Buffer.cpp
  ${V}/src/common/ConfigManager.cpp
  ${V}/src/common/Patch.cpp
  ${V}/src/common/dictionary.c
  ${V}/src/common/iniparser.c
  ${V}/src/common/memgzio.c
  ${V}/src/gb/GB.cpp
  ${V}/src/gb/gbCheats.cpp
  ${V}/src/gb/gbDis.cpp
  ${V}/src/gb/gbGfx.cpp
  ${V}/src/gb/gbGlobals.cpp
  ${V}/src/gb/gbMemory.cpp
  ${V}/src/gb/gbPrinter.cpp
  ${V}/src/gb/gbSGB.cpp
  ${V}/src/gb/gbSound.cpp
  ${V}/src/gba/BreakpointStructures.cpp
  ${V}/src/gba/CheatSearch.cpp
  ${V}/src/gba/Cheats.cpp
  ${V}/src/gba/EEprom.cpp
  ${V}/src/gba/Flash.cpp
  ${V}/src/gba/GBA-arm.cpp
  ${V}/src/gba/GBA-thumb.cpp
  ${V}/src/gba/GBA.cpp
  ${V}/src/gba/GBAGfx.cpp
  ${V}/src/gba/GBALink.cpp
  ${V}/src/gba/GBASockClient.cpp
  ${V}/src/gba/Globals.cpp
  ${V}/src/gba/Mode0.cpp
  ${V}/src/gba/Mode1.cpp
  ${V}/src/gba/Mode2.cpp
  ${V}/src/gba/Mode3.cpp
  ${V}/src/gba/Mode4.cpp
  ${V}/src/gba/Mode5.cpp
  ${V}/src/gba/RTC.cpp
  ${V}/src/gba/Sound.cpp
  ${V}/src/gba/Sram.cpp
  ${V}/src/gba/agbprint.cpp
  ${V}/src/gba/armdis.cpp
  ${V}/src/gba/bios.cpp
  ${V}/src/gba/debugger-expr-lex.cpp
  ${V}/src/gba/debugger-expr-yacc.cpp
  ${V}/src/gba/elf.cpp
  ${V}/src/gba/ereader.cpp
  ${V}/src/gba/gbafilter.cpp
  ${V}/src/gba/remote.cpp
  ${V}/fex/7z_C/7zAlloc.c
  ${V}/fex/7z_C/7zBuf.c
  ${V}/fex/7z_C/7zCrc.c
  ${V}/fex/7z_C/7zCrcOpt.c
  ${V}/fex/7z_C/7zDec.c
  ${V}/fex/7z_C/7zIn.c
  ${V}/fex/7z_C/7zStream.c
  ${V}/fex/7z_C/Bcj2.c
  ${V}/fex/7z_C/Bra.c
  ${V}/fex/7z_C/Bra86.c
  ${V}/fex/7z_C/CpuArch.c
  ${V}/fex/7z_C/Lzma2Dec.c
  ${V}/fex/7z_C/LzmaDec.c
  ${V}/fex/7z_C/Ppmd7.c
  ${V}/fex/7z_C/Ppmd7Dec.c
  ${V}/fex/fex/Binary_Extractor.cpp
  ${V}/fex/fex/Data_Reader.cpp
  ${V}/fex/fex/File_Extractor.cpp
  ${V}/fex/fex/Gzip_Extractor.cpp
  ${V}/fex/fex/Gzip_Reader.cpp
  ${V}/fex/fex/Rar_Extractor.cpp
  ${V}/fex/fex/Zip7_Extractor.cpp
  ${V}/fex/fex/Zip_Extractor.cpp
  ${V}/fex/fex/Zlib_Inflater.cpp
  ${V}/fex/fex/blargg_common.cpp
  ${V}/fex/fex/blargg_errors.cpp
  ${V}/fex/fex/fex.cpp
  # SFML System and Network, for VBA-M's link cable code.
  ${SFML}/src/SFML/System/Err.cpp
  ${SFML}/src/SFML/System/String.cpp
  ${SFML}/src/SFML/System/Thread.cpp
  ${SFML}/src/SFML/System/Time.cpp
  ${SFML}/src/SFML/System/Unix/ThreadImpl.cpp
  ${SFML}/src/SFML/Network/Ftp.cpp
  ${SFML}/src/SFML/Network/Http.cpp
  ${SFML}/src/SFML/Network/IpAddress.cpp
  ${SFML}/src/SFML/Network/Packet.cpp
  ${SFML}/src/SFML/Network/Socket.cpp
  ${SFML}/src/SFML/Network/SocketSelector.cpp
  ${SFML}/src/SFML/Network/TcpListener.cpp
  ${SFML}/src/SFML/Network/TcpSocket.cpp
  ${SFML}/src/SFML/Network/UdpSocket.cpp
  ${SFML}/src/SFML/Network/Unix/SocketImpl.cpp
)
set_target_properties(vbam-harness PROPERTIES C_STANDARD 99 C_EXTENSIONS ON CXX_STANDARD 11 CXX_EXTENSIONS ON)
# GBADeltaCore's OTHER_CFLAGS, which Xcode applies to C and C++ alike.
target_compile_definitions(vbam-harness PRIVATE
  C_CORE NO_PNG FINAL_VERSION PKGDATADIR SYSCONF_INSTALL_DIR NO_DEBUGGER BKPT_SUPPORT HAVE_ARPA_INET_H)
target_compile_options(vbam-harness PRIVATE ${VBAM_OPT} -ffast-math -w)
target_include_directories(vbam-harness PRIVATE ${V}/src ${V}/fex ${SFML}/include ${SFML}/src)
target_link_libraries(vbam-harness PRIVATE ZLIB::ZLIB pthread)
