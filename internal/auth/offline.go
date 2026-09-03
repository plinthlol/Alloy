package auth

import (
	"crypto/md5"
)

func OfflineUUID(username string) [16]byte {
	sum := md5.Sum([]byte("OfflinePlayer:" + username))
	sum[6] = (sum[6] & 0x0F) | 0x30
	sum[8] = (sum[8] & 0x3F) | 0x80
	return sum
}

func OfflineUUIDString(username string) string {
	b := OfflineUUID(username)
	return formatUUID(b)
}

func formatUUID(b [16]byte) string {
	const hexdigits = "0123456789abcdef"
	buf := make([]byte, 36)
	pos := 0
	dashAfter := map[int]bool{4: true, 6: true, 8: true, 10: true}
	for i, v := range b {
		buf[pos] = hexdigits[v>>4]
		buf[pos+1] = hexdigits[v&0x0F]
		pos += 2
		if dashAfter[i+1] {
			buf[pos] = '-'
			pos++
		}
	}
	return string(buf[:pos])
}

type OfflineProfile struct {
	Username string
	UUID     string
}

func NewOfflineProfile(username string) OfflineProfile {
	return OfflineProfile{
		Username: username,
		UUID:     OfflineUUIDString(username),
	}
}
