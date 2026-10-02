using System.Text;

namespace VstApplier;

public sealed class VstPluginScanner
{
    private const ushort PeMachineAmd64 = 0x8664;
    private const ushort Pe32Magic = 0x10b;
    private const ushort Pe32PlusMagic = 0x20b;

    public IReadOnlyList<VstPluginCandidate> Scan(string folderPath)
    {
        if (string.IsNullOrWhiteSpace(folderPath) || !Directory.Exists(folderPath))
        {
            return [];
        }

        var plugins = new List<VstPluginCandidate>();

        plugins.AddRange(Directory
            .EnumerateFileSystemEntries(folderPath, "*.vst3", SearchOption.AllDirectories)
            .Where(path => File.Exists(path) || Directory.Exists(path))
            .Select(path => new VstPluginCandidate(
                path,
                Path.GetFileNameWithoutExtension(path),
                VstPluginFormat.Vst3)));

        plugins.AddRange(Directory
            .EnumerateFiles(folderPath, "*.dll", SearchOption.AllDirectories)
            .Where(IsVst2X64Plugin)
            .Select(path => new VstPluginCandidate(
                path,
                Path.GetFileNameWithoutExtension(path),
                VstPluginFormat.Vst2)));

        return plugins
            .OrderBy(plugin => plugin.Format)
            .ThenBy(plugin => plugin.Name, StringComparer.OrdinalIgnoreCase)
            .ToList();
    }

    private static bool IsVst2X64Plugin(string path)
    {
        try
        {
            using var stream = File.OpenRead(path);
            using var reader = new BinaryReader(stream);

            if (stream.Length < 0x40 || reader.ReadUInt16() != 0x5A4D)
            {
                return false;
            }

            stream.Position = 0x3C;
            var peHeaderOffset = reader.ReadInt32();
            if (peHeaderOffset <= 0 || peHeaderOffset > stream.Length - 0x108)
            {
                return false;
            }

            stream.Position = peHeaderOffset;
            if (reader.ReadUInt32() != 0x00004550)
            {
                return false;
            }

            var machine = reader.ReadUInt16();
            var sectionCount = reader.ReadUInt16();
            stream.Position += 12;
            var optionalHeaderSize = reader.ReadUInt16();
            stream.Position += 2;

            if (machine != PeMachineAmd64 || sectionCount <= 0 || optionalHeaderSize <= 0)
            {
                return false;
            }

            var optionalHeaderOffset = stream.Position;
            var optionalHeaderMagic = reader.ReadUInt16();
            var exportDataDirectoryOffset = optionalHeaderMagic switch
            {
                Pe32Magic => optionalHeaderOffset + 0x60,
                Pe32PlusMagic => optionalHeaderOffset + 0x70,
                _ => -1,
            };

            if (exportDataDirectoryOffset < 0 ||
                exportDataDirectoryOffset + 8 > optionalHeaderOffset + optionalHeaderSize)
            {
                return false;
            }

            stream.Position = exportDataDirectoryOffset;
            var exportTableRva = reader.ReadUInt32();
            if (exportTableRva == 0)
            {
                return false;
            }

            var sections = ReadSections(reader, optionalHeaderOffset + optionalHeaderSize, sectionCount);
            var exportTableOffset = RvaToFileOffset(exportTableRva, sections);
            if (exportTableOffset < 0 || exportTableOffset + 40 > stream.Length)
            {
                return false;
            }

            stream.Position = exportTableOffset + 24;
            var exportedNameCount = reader.ReadUInt32();
            stream.Position += 4;
            var exportedNameRvas = reader.ReadUInt32();
            var exportedNamesOffset = RvaToFileOffset(exportedNameRvas, sections);

            if (exportedNameCount == 0 || exportedNamesOffset < 0)
            {
                return false;
            }

            for (var index = 0; index < exportedNameCount; index++)
            {
                var nameRvaPointerOffset = exportedNamesOffset + index * 4;
                if (nameRvaPointerOffset < 0 || nameRvaPointerOffset + 4 > stream.Length)
                {
                    return false;
                }

                stream.Position = nameRvaPointerOffset;
                var nameRva = reader.ReadUInt32();
                var nameOffset = RvaToFileOffset(nameRva, sections);
                if (nameOffset < 0 || nameOffset >= stream.Length)
                {
                    continue;
                }

                var exportName = ReadAsciiNullTerminated(stream, nameOffset);
                if (exportName is "VSTPluginMain" or "main")
                {
                    return true;
                }
            }
        }
        catch
        {
            return false;
        }

        return false;
    }

    private static IReadOnlyList<PeSection> ReadSections(BinaryReader reader, long sectionTableOffset, int sectionCount)
    {
        var sections = new List<PeSection>(sectionCount);
        reader.BaseStream.Position = sectionTableOffset;

        for (var index = 0; index < sectionCount; index++)
        {
            reader.BaseStream.Position += 8;
            var virtualSize = reader.ReadUInt32();
            var virtualAddress = reader.ReadUInt32();
            var rawDataSize = reader.ReadUInt32();
            var rawDataPointer = reader.ReadUInt32();
            reader.BaseStream.Position += 16;

            sections.Add(new PeSection(
                virtualAddress,
                Math.Max(virtualSize, rawDataSize),
                rawDataPointer));
        }

        return sections;
    }

    private static long RvaToFileOffset(uint rva, IReadOnlyList<PeSection> sections)
    {
        foreach (var section in sections)
        {
            var sectionEnd = section.VirtualAddress + section.Size;
            if (rva >= section.VirtualAddress && rva < sectionEnd)
            {
                return section.RawDataPointer + rva - section.VirtualAddress;
            }
        }

        return -1;
    }

    private static string ReadAsciiNullTerminated(Stream stream, long offset)
    {
        stream.Position = offset;
        var bytes = new List<byte>();

        while (stream.Position < stream.Length)
        {
            var value = stream.ReadByte();
            if (value <= 0)
            {
                break;
            }

            bytes.Add((byte)value);
        }

        return Encoding.ASCII.GetString(bytes.ToArray());
    }

    private readonly record struct PeSection(uint VirtualAddress, uint Size, uint RawDataPointer);
}
