import Foundation

public class BinaryDecoder {
    func decodeQRErrorCorrectedBytes(_ errorCorrectedPayload: Data, symbolVersion: Int) -> Data? {
        let binData = Binary(data: errorCorrectedPayload)
        guard let decodedData = decode(binData, symbolVersion: symbolVersion) else {
            return nil
        }

        return decodedData
    }

    private func decode(_ binary: Binary, symbolVersion: Int) -> Data? {
        let numberOfBitsPerCharacter = 8
        var binaryData = binary

        guard let numberOfBitsInLengthFiled = numberOfBitsInLengthFiled(for: symbolVersion) else {
            return nil
        }

        let totalCharacterCount = binaryData.next(bits: numberOfBitsInLengthFiled)
        var bytes: [UInt8] = []
        for _ in .zero ..< totalCharacterCount {
            let byte = binaryData.next(bits: numberOfBitsPerCharacter)
            bytes.append(UInt8(byte))
        }

        return Data(bytes)
    }

    private func numberOfBitsInLengthFiled(for symbolVersion: Int) -> Int? {
        guard let symbolType = SymbolType(version: symbolVersion) else {
            return nil
        }

        switch symbolType {
        case .small:
            return 8
        case .medium:
            return 16
        case .large:
            return 16
        }
    }
}

extension BinaryDecoder {
    private enum SymbolType {
        case small
        case medium
        case large

        init?(
            version: Int
        ) {
            if version >= 1, version <= 9 {
                self = .small
            } else if version >= 10, version <= 26 {
                self = .medium
            } else if version >= 27, version <= 40 {
                self = .large
            } else {
                return nil
            }
        }
    }

    private struct Binary {
        private let bytes: [UInt8]
        private var readingOffset: Int = 4

        init(
            bytes: [UInt8]
        ) {
            self.bytes = bytes
        }

        init(
            data: Data
        ) {
            let bytesLength = data.count
            var bytesArray = [UInt8](repeating: .zero, count: bytesLength)
            (data as NSData).getBytes(&bytesArray, length: bytesLength)
            bytes = bytesArray
        }

        private func bit(_ position: Int) -> Int {
            let byteSize = 8
            let bytePosition = position / byteSize
            let bitPosition = 7 - (position % byteSize)
            let byte = self.byte(bytePosition)
            return (byte >> bitPosition) & 0x01
        }

        private func bits(_ range: Range<Int>) -> Int {
            var positions = [Int]()

            for position in range.lowerBound ..< range.upperBound {
                positions.append(position)
            }

            return positions.reversed().enumerated().reduce(0) {
                $0 + (self.bit($1.element) << $1.offset)
            }
        }

        private func bits(_ start: Int, _ length: Int) -> Int {
            return bits(start ..< (start + length))
        }

        private func byte(_ position: Int) -> Int {
            return Int(bytes[position])
        }

        private func bitsWithInternalOffsetAvailable(_ length: Int) -> Bool {
            return (bytes.count * 8) >= (readingOffset + length)
        }

        mutating func next(bits length: Int) -> Int {
            if bitsWithInternalOffsetAvailable(length) {
                let returnValue = bits(readingOffset, length)
                readingOffset = readingOffset + length
                return returnValue
            } else {
                fatalError("Couldn't extract Bits.")
            }
        }

        private func bytesWithInternalOffsetAvailable(_ length: Int) -> Bool {
            let availableBits = bytes.count * 8
            let requestedBits = readingOffset + (length * 8)
            let possible = availableBits >= requestedBits
            return possible
        }
    }
}
