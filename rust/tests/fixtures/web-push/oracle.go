//go:build ignore

package main
import (
	"bytes"
	"context"
	"crypto/aes"
	"crypto/cipher"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"errors"
	"io"
	"net/http"
	"strconv"
	"strings"
	"time"

	"golang.org/x/crypto/hkdf"
)
import("os";"encoding/json")
type Urgency string
func isValidUrgency(Urgency)bool{return false}
const MaxRecordSize uint32=4096
var ErrMaxPadExceeded=errors.New("payload has exceeded the maximum length")
var saltFunc=func()([]byte,error){return make([]byte,16),nil}
func getVAPIDAuthorizationHeader(string,string,string,string,time.Time)(string,error){return "synthetic-test-only",nil}
var _=rand.Reader
type HTTPClient interface {
	Do(*http.Request) (*http.Response, error)
}
type Options struct {
	HTTPClient      HTTPClient // Will replace with *http.Client by default if not included
	RecordSize      uint32     // Limit the record size
	Subscriber      string     // Sub in VAPID JWT token
	Topic           string     // Set the Topic header to collapse a pending messages (Optional)
	TTL             int        // Set the TTL on the endpoint POST request
	Urgency         Urgency    // Set the Urgency header to change a message priority (Optional)
	VAPIDPublicKey  string     // VAPID public key, passed in VAPID Authorization header
	VAPIDPrivateKey string     // VAPID private key, used to sign VAPID JWT token
	VapidExpiration time.Time  // optional expiration for VAPID JWT token (defaults to now + 12 hours)
}
type Keys struct {
	Auth   string `json:"auth"`
	P256dh string `json:"p256dh"`
}
type Subscription struct {
	Endpoint string `json:"endpoint"`
	Keys     Keys   `json:"keys"`
}
func SendNotificationWithContext(ctx context.Context, message []byte, s *Subscription, options *Options) (*http.Response, error) {
	// Authentication secret (auth_secret)
	authSecret, err := decodeSubscriptionKey(s.Keys.Auth)
	if err != nil {
		return nil, err
	}

	// dh (Diffie Hellman)
	dh, err := decodeSubscriptionKey(s.Keys.P256dh)
	if err != nil {
		return nil, err
	}

	// Generate 16 byte salt
	salt, err := saltFunc()
	if err != nil {
		return nil, err
	}

	// Create the ecdh_secret shared key pair
	curve := elliptic.P256()

	// Application server key pairs (single use)
	localPrivateKey := make([]byte,32); localPrivateKey[31]=2; x,y := curve.ScalarBaseMult(localPrivateKey)
	if err != nil {
		return nil, err
	}

	localPublicKey := elliptic.Marshal(curve, x, y)

	// Combine application keys with receiver's EC public key
	sharedX, sharedY := elliptic.Unmarshal(curve, dh)
	if sharedX == nil {
		return nil, errors.New("Unmarshal Error: Public key is not a valid point on the curve")
	}

	// Derive ECDH shared secret
	sx, sy := curve.ScalarMult(sharedX, sharedY, localPrivateKey)
	if !curve.IsOnCurve(sx, sy) {
		return nil, errors.New("Encryption error: ECDH shared secret isn't on curve")
	}
	mlen := curve.Params().BitSize / 8
	sharedECDHSecret := make([]byte, mlen)
	sx.FillBytes(sharedECDHSecret)

	hash := sha256.New

	// ikm
	prkInfoBuf := bytes.NewBuffer([]byte("WebPush: info\x00"))
	prkInfoBuf.Write(dh)
	prkInfoBuf.Write(localPublicKey)

	prkHKDF := hkdf.New(hash, sharedECDHSecret, authSecret, prkInfoBuf.Bytes())
	ikm, err := getHKDFKey(prkHKDF, 32)
	if err != nil {
		return nil, err
	}

	// Derive Content Encryption Key
	contentEncryptionKeyInfo := []byte("Content-Encoding: aes128gcm\x00")
	contentHKDF := hkdf.New(hash, ikm, salt, contentEncryptionKeyInfo)
	contentEncryptionKey, err := getHKDFKey(contentHKDF, 16)
	if err != nil {
		return nil, err
	}

	// Derive the Nonce
	nonceInfo := []byte("Content-Encoding: nonce\x00")
	nonceHKDF := hkdf.New(hash, ikm, salt, nonceInfo)
	nonce, err := getHKDFKey(nonceHKDF, 12)
	if err != nil {
		return nil, err
	}

	// Cipher
	c, err := aes.NewCipher(contentEncryptionKey)
	if err != nil {
		return nil, err
	}

	gcm, err := cipher.NewGCM(c)
	if err != nil {
		return nil, err
	}

	// Get the record size
	recordSize := options.RecordSize
	if recordSize == 0 {
		recordSize = MaxRecordSize
	}

	recordLength := int(recordSize) - 16

	// Encryption Content-Coding Header
	recordBuf := bytes.NewBuffer(salt)

	rs := make([]byte, 4)
	binary.BigEndian.PutUint32(rs, recordSize)

	recordBuf.Write(rs)
	recordBuf.Write([]byte{byte(len(localPublicKey))})
	recordBuf.Write(localPublicKey)

	// Data
	dataBuf := bytes.NewBuffer(message)

	// Pad content to max record size - 16 - header
	// Padding ending delimeter
	dataBuf.Write([]byte("\x02"))
	if err := pad(dataBuf, recordLength-recordBuf.Len()); err != nil {
		return nil, err
	}

	// Compose the ciphertext
	ciphertext := gcm.Seal([]byte{}, nonce, dataBuf.Bytes(), nil)
	recordBuf.Write(ciphertext)

	// POST request
	req, err := http.NewRequest("POST", s.Endpoint, recordBuf)
	if err != nil {
		return nil, err
	}

	if ctx != nil {
		req = req.WithContext(ctx)
	}

	req.Header.Set("Content-Encoding", "aes128gcm")
	req.Header.Set("Content-Type", "application/octet-stream")
	req.Header.Set("TTL", strconv.Itoa(options.TTL))

	// Сheck the optional headers
	if len(options.Topic) > 0 {
		req.Header.Set("Topic", options.Topic)
	}

	if isValidUrgency(options.Urgency) {
		req.Header.Set("Urgency", string(options.Urgency))
	}

	expiration := options.VapidExpiration
	if expiration.IsZero() {
		expiration = time.Now().Add(time.Hour * 12)
	}

	// Get VAPID Authorization header
	vapidAuthHeader, err := getVAPIDAuthorizationHeader(
		s.Endpoint,
		options.Subscriber,
		options.VAPIDPublicKey,
		options.VAPIDPrivateKey,
		expiration,
	)
	if err != nil {
		return nil, err
	}

	req.Header.Set("Authorization", vapidAuthHeader)

	// Send the request
	var client HTTPClient
	if options.HTTPClient != nil {
		client = options.HTTPClient
	} else {
		client = &http.Client{}
	}

	return client.Do(req)
}

func decodeSubscriptionKey(key string) ([]byte, error) {
	// "=" padding
	buf := bytes.NewBufferString(key)
	if rem := len(key) % 4; rem != 0 {
		buf.WriteString(strings.Repeat("=", 4-rem))
	}

	bytes, err := base64.StdEncoding.DecodeString(buf.String())
	if err == nil {
		return bytes, nil
	}

	return base64.URLEncoding.DecodeString(buf.String())
}

func getHKDFKey(hkdf io.Reader, length int) ([]byte, error) {
	key := make([]byte, length)
	n, err := io.ReadFull(hkdf, key)
	if n != len(key) || err != nil {
		return key, err
	}

	return key, nil
}

func pad(payload *bytes.Buffer, maxPadLen int) error {
	payloadLen := payload.Len()
	if payloadLen > maxPadLen {
		return ErrMaxPadExceeded
	}

	padLen := maxPadLen - payloadLen

	padding := make([]byte, padLen)
	payload.Write(padding)

	return nil
}

type Capture struct{Body string}
func(c *Capture)Do(r *http.Request)(*http.Response,error){data,e:=io.ReadAll(r.Body);if e!=nil{return nil,e};c.Body=base64.RawURLEncoding.EncodeToString(data);return &http.Response{StatusCode:201,Body:http.NoBody},nil}
func main(){private:=make([]byte,32);private[31]=1;x,y:=elliptic.P256().ScalarBaseMult(private);keys:=Keys{P256dh:base64.RawURLEncoding.EncodeToString(elliptic.Marshal(elliptic.P256(),x,y)),Auth:base64.RawURLEncoding.EncodeToString(make([]byte,16))};rows:=[]any{};for _,payload:=range []string{"synthetic push payload", "合成 Web Push だけです"}{client:=&Capture{};_,e:=SendNotificationWithContext(context.Background(),[]byte(payload),&Subscription{Endpoint:"https://example.invalid/push",Keys:keys},&Options{HTTPClient:client,TTL:300});if e!=nil{panic(e)};rows=append(rows,map[string]string{"payload":payload,"body":client.Body,"p256dh":keys.P256dh,"auth":keys.Auth})};data,e:=json.MarshalIndent(rows,"","  ");if e!=nil{panic(e)};if e=os.WriteFile(os.Args[1],data,0600);e!=nil{panic(e)}}
